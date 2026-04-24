mod boundary;
mod embedding_stats;
mod review;

use chrono::TimeDelta;
use plastmem_ai::embed_many;
use plastmem_event::{Event, EventDataToString};
use plastmem_shared::AppError;

use crate::{EventSegment, EventSegmentReason};

use self::{
  boundary::{BoundaryCandidate, boundary_budget, collect_candidates},
  embedding_stats::{build_prefix, segment_cohesion},
  review::{review_candidates_with_llm, review_short_segments_with_llm},
};

pub struct EventSegmenter;

const SOFT_TIME_GAP: TimeDelta = TimeDelta::minutes(30);
const HARD_TIME_GAP: TimeDelta = TimeDelta::hours(3);
const MIN_SEGMENT_EVENTS: usize = 4;
const REVIEW_CONTEXT_EVENTS: usize = 5;
const MAX_REVIEW_CANDIDATES: usize = 40;
const TARGET_EVENTS_PER_SEGMENT: usize = 12;

impl EventSegmenter {
  pub async fn segment(events: &[Event]) -> Result<Vec<EventSegment>, AppError> {
    if events.is_empty() {
      return Ok(Vec::new());
    }

    let inputs = events
      .iter()
      .map(|event| event.data.to_string_without_timestamp())
      .collect::<Vec<_>>();
    let embeddings = embed_many(&inputs)
      .await?
      .into_iter()
      .map(|embedding| embedding.to_vec())
      .collect::<Vec<_>>();

    Ok(Self::segment_with_embeddings(events, &embeddings).await)
  }

  async fn segment_with_embeddings(events: &[Event], embeddings: &[Vec<f32>]) -> Vec<EventSegment> {
    let mut segments = Vec::new();
    let mut start = 0usize;

    for end in 1..=events.len() {
      if !is_partition_end(events, end) {
        continue;
      }

      let partition_segments = Self::segment_partition(
        &events[start..end],
        &embeddings[start..end],
        if start > 0 {
          EventSegmentReason::HardTimeGap
        } else {
          EventSegmentReason::InitialSegment
        },
      )
      .await;
      segments.extend(partition_segments);
      start = end;
    }

    segments
  }

  async fn segment_partition(
    events: &[Event],
    embeddings: &[Vec<f32>],
    start_reason: EventSegmentReason,
  ) -> Vec<EventSegment> {
    if events.len() <= 1 {
      return vec![EventSegment::with_metadata(
        events.to_vec(),
        start_reason,
        1.0,
        1.0,
        1.0,
      )];
    }

    let prefix = build_prefix(embeddings);
    let candidates = collect_candidates(events, embeddings, &prefix);
    let reviewed = reviewed_boundaries(events, candidates).await;

    if reviewed.is_empty() {
      return vec![leaf_segment(events, embeddings, &prefix, start_reason)];
    }

    let coarse_segments = build_segments(events, embeddings, &prefix, &reviewed);
    let refined = Self::refine_segments(events, embeddings, &coarse_segments, start_reason).await;

    match review_short_segments_with_llm(&refined).await {
      Ok(segments) => segments,
      Err(err) => {
        tracing::warn!(error = %err, "Short segment merge review failed; keeping reviewed boundaries");
        refined
      }
    }
  }

  async fn refine_segments(
    events: &[Event],
    embeddings: &[Vec<f32>],
    coarse_segments: &[EventSegment],
    start_reason: EventSegmentReason,
  ) -> Vec<EventSegment> {
    let mut refined = Vec::new();
    let mut offset = 0usize;

    for (index, coarse_segment) in coarse_segments.iter().enumerate() {
      let segment_len = coarse_segment.events.len();
      refined.extend(
        Box::pin(Self::segment_partition(
          &events[offset..offset + segment_len],
          &embeddings[offset..offset + segment_len],
          if index == 0 {
            start_reason
          } else {
            coarse_segment.reason
          },
        ))
        .await,
      );
      offset += segment_len;
    }

    refined
  }
}

async fn reviewed_boundaries(
  events: &[Event],
  candidates: Vec<BoundaryCandidate>,
) -> Vec<BoundaryCandidate> {
  match review_candidates_with_llm(events, &candidates).await {
    Ok(reviewed) => reviewed,
    Err(err) => {
      tracing::warn!(error = %err, "Boundary review failed; using embedding candidates");
      candidates
        .into_iter()
        .take(boundary_budget(events.len()))
        .collect()
    }
  }
}

fn is_partition_end(events: &[Event], end: usize) -> bool {
  end == events.len() || time_gap_before(events, end) > HARD_TIME_GAP
}

fn leaf_segment(
  events: &[Event],
  embeddings: &[Vec<f32>],
  prefix: &[Vec<f32>],
  reason: EventSegmentReason,
) -> EventSegment {
  EventSegment::with_metadata(
    events.to_vec(),
    reason,
    segment_cohesion(embeddings, prefix, 0, events.len()),
    1.0,
    1.0,
  )
}

fn build_segments(
  events: &[Event],
  embeddings: &[Vec<f32>],
  prefix: &[Vec<f32>],
  boundaries: &[BoundaryCandidate],
) -> Vec<EventSegment> {
  let mut result = Vec::new();
  let mut start = 0usize;
  let points = boundaries
    .iter()
    .map(|candidate| candidate.index)
    .chain(std::iter::once(events.len()));

  for end in points {
    if start >= end {
      continue;
    }
    result.push(EventSegment::with_metadata(
      events[start..end].to_vec(),
      boundary::reason_for(boundaries, start),
      segment_cohesion(embeddings, prefix, start, end),
      boundary::confidence_for(boundaries, start),
      boundary::confidence_for(boundaries, end),
    ));
    start = end;
  }

  result
}

fn time_gap_before(events: &[Event], index: usize) -> TimeDelta {
  if index == 0 || index >= events.len() {
    return TimeDelta::zero();
  }
  events[index]
    .timestamp
    .signed_duration_since(events[index - 1].timestamp)
}

#[cfg(test)]
mod tests {
  use chrono::{TimeZone, Utc};
  use plastmem_event::{Event, EventData, MessageEventData, MessageEventRole};

  use super::*;
  use crate::event_segmenter::embedding_stats::normalize;

  fn message_event(id: u128, minute: i64, content: &str) -> Event {
    Event::new(
      EventData::Message(MessageEventData {
        role: MessageEventRole::User,
        content: content.to_owned(),
      }),
      Utc
        .timestamp_opt(1_700_000_000 + minute * 60, 0)
        .single()
        .expect("valid timestamp"),
      Some(uuid::Uuid::from_u128(id)),
    )
  }

  fn embedding(x: f32, y: f32) -> Vec<f32> {
    let mut value = vec![x, y];
    normalize(&mut value);
    value
  }

  #[test]
  fn embedding_candidates_find_obvious_topic_shift() {
    let events = (0..8)
      .map(|index| message_event(index + 1, index as i64, "message"))
      .collect::<Vec<_>>();
    let embeddings = vec![
      embedding(1.0, 0.0),
      embedding(1.0, 0.0),
      embedding(1.0, 0.0),
      embedding(1.0, 0.0),
      embedding(0.0, 1.0),
      embedding(0.0, 1.0),
      embedding(0.0, 1.0),
      embedding(0.0, 1.0),
    ];
    let prefix = build_prefix(&embeddings);
    let candidates = collect_candidates(&events, &embeddings, &prefix);
    let segments = build_segments(&events, &embeddings, &prefix, &candidates);

    assert_eq!(
      candidates
        .iter()
        .map(|candidate| candidate.index)
        .collect::<Vec<_>>(),
      vec![4]
    );
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[1].reason, EventSegmentReason::TopicShift);
  }

  #[tokio::test]
  async fn hard_time_gap_forces_partition_boundary() {
    let events = vec![
      message_event(1, 0, "a"),
      message_event(2, 1, "a"),
      message_event(3, 240, "a"),
      message_event(4, 241, "a"),
    ];
    let embeddings = vec![
      embedding(1.0, 0.0),
      embedding(1.0, 0.0),
      embedding(1.0, 0.0),
      embedding(1.0, 0.0),
    ];

    let segments = EventSegmenter::segment_with_embeddings(&events, &embeddings).await;

    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0].reason, EventSegmentReason::InitialSegment);
    assert_eq!(segments[1].reason, EventSegmentReason::HardTimeGap);
  }
}
