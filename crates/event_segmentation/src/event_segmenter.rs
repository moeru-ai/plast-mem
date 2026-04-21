use anyhow::anyhow;
use chrono::TimeDelta;
use plastmem_ai::{
  ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
  ChatCompletionRequestUserMessage, generate_object,
};
use plastmem_event::Event;
use plastmem_shared::AppError;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::prompt::{large_segment_split_prompt, small_segment_merge_prompt};
use crate::{EventSegment, EventSegmentReason};

pub struct EventSegmenter {}

#[derive(Debug, Deserialize, JsonSchema)]
struct SmallSegmentMergeOutput {
  merge_with_previous: bool,
  reason_if_separate: EventSegmentReason,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct LargeSegmentSplitOutput {
  split_start_event_indices: Vec<u32>,
  boundary_reasons: Vec<EventSegmentReason>,
}

impl EventSegmenter {
  const TIME_GAP_THRESHOLD: TimeDelta = TimeDelta::minutes(30);
  const HARD_TIME_GAP_THRESHOLD: TimeDelta = TimeDelta::hours(3);
  const SMALL_SEGMENT_MAX_EVENTS: usize = 4;
  const LARGE_SEGMENT_SPLIT_TRIGGER: usize = 20;

  // Perform segmented processing on events with intervals exceeding 30 minutes.
  fn segment_by_time_gap(events: &[Event]) -> Result<Vec<EventSegment>, AppError> {
    if events.is_empty() {
      return Ok(Vec::new());
    }

    let mut segments = Vec::new();
    let mut curr_events = vec![events[0].clone()];
    let mut curr_reasons = Vec::new();
    let mut prev = &events[0];

    for curr in events.iter().skip(1) {
      let gap = curr.timestamp.signed_duration_since(prev.timestamp);

      if gap > Self::TIME_GAP_THRESHOLD {
        segments.push(EventSegment::new(
          std::mem::take(&mut curr_events),
          std::mem::take(&mut curr_reasons),
        ));
        curr_events.push(curr.clone());
        curr_reasons.push(Self::time_gap_reason(gap));
      } else {
        curr_events.push(curr.clone());
      }

      prev = curr;
    }

    segments.push(EventSegment::new(curr_events, curr_reasons));
    Ok(segments)
  }

  async fn segment_by_llm(
    input_segments: Vec<EventSegment>,
  ) -> Result<Vec<EventSegment>, AppError> {
    let mut segments = Vec::new();

    for segment in input_segments {
      if segment.events.is_empty() {
        continue;
      }

      for segment in Self::check_large_segment(segment).await {
        if let Some(previous) = segments.last_mut() {
          if let Some(segment) = Self::check_small_segment(previous, segment).await? {
            segments.push(segment);
          }
        } else {
          segments.push(segment);
        }
      }
    }

    Ok(segments)
  }

  async fn check_large_segment(segment: EventSegment) -> Vec<EventSegment> {
    if segment.events.len() <= Self::LARGE_SEGMENT_SPLIT_TRIGGER {
      return vec![segment];
    }

    let reason = segment.reasons.first().copied();
    let split_segments = Self::try_split_large_segment(&segment.events).await;

    if split_segments.is_empty() {
      return vec![EventSegment::new(
        segment.events,
        reason.into_iter().collect(),
      )];
    }

    split_segments
      .into_iter()
      .enumerate()
      .map(|(index, segment)| {
        if index == 0 {
          segment.prepend_reason_if_missing(reason)
        } else {
          segment
        }
      })
      .collect()
  }

  async fn check_small_segment(
    previous: &mut EventSegment,
    segment: EventSegment,
  ) -> Result<Option<EventSegment>, AppError> {
    if segment.events.len() > Self::SMALL_SEGMENT_MAX_EVENTS {
      return Ok(Some(segment));
    }

    if segment.reasons.contains(&EventSegmentReason::HardTimeGap) {
      return Ok(Some(segment));
    }

    let merge_output = Self::should_merge_small_segment(&previous.events, &segment.events)
      .await
      .unwrap_or(SmallSegmentMergeOutput {
        merge_with_previous: false,
        reason_if_separate: EventSegmentReason::TopicShift,
      });

    if !merge_output.merge_with_previous {
      return Ok(Some(
        segment.replace_reason(merge_output.reason_if_separate),
      ));
    }

    previous.extend_events(segment.events);
    Ok(None)
  }

  async fn should_merge_small_segment(
    previous_events: &[Event],
    current_events: &[Event],
  ) -> Result<SmallSegmentMergeOutput, AppError> {
    let prompt = small_segment_merge_prompt(previous_events, current_events);
    let system = ChatCompletionRequestSystemMessage::from(prompt.system);
    let user = ChatCompletionRequestUserMessage::from(prompt.user);

    let output = generate_object::<SmallSegmentMergeOutput>(
      vec![
        ChatCompletionRequestMessage::System(system),
        ChatCompletionRequestMessage::User(user),
      ],
      "event_small_segment_merge".to_owned(),
      Some(
        "Decide whether a small event segment should merge into the previous segment".to_owned(),
      ),
    )
    .await?;

    if output.reason_if_separate.is_time_gap() {
      return Err(AppError::new(anyhow!(
        "Small segment merge output cannot use time gap as a semantic boundary reason"
      )));
    }

    Ok(output)
  }

  async fn split_large_segment(events: &[Event]) -> Result<Vec<EventSegment>, AppError> {
    let prompt = large_segment_split_prompt(events);
    let system = ChatCompletionRequestSystemMessage::from(prompt.system);
    let user = ChatCompletionRequestUserMessage::from(prompt.user);

    let output = generate_object::<LargeSegmentSplitOutput>(
      vec![
        ChatCompletionRequestMessage::System(system),
        ChatCompletionRequestMessage::User(user),
      ],
      "event_large_segment_split".to_owned(),
      Some("Split a large event segment into topic-consistent event segments".to_owned()),
    )
    .await?;

    Self::resolve_large_segment_split(events, output)
      .map_err(|reason| AppError::new(anyhow!(reason)))
  }

  async fn try_split_large_segment(events: &[Event]) -> Vec<EventSegment> {
    match Self::split_large_segment(events).await {
      Ok(segments) => segments,
      Err(err) => {
        tracing::warn!(
          event_count = events.len(),
          error = %err,
          "Large event segment split failed; falling back to one segment"
        );
        vec![EventSegment::new(events.to_vec(), Vec::new())]
      }
    }
  }

  fn resolve_large_segment_split(
    events: &[Event],
    output: LargeSegmentSplitOutput,
  ) -> Result<Vec<EventSegment>, String> {
    let split_indices =
      Self::validate_split_indices(events.len(), &output.split_start_event_indices)?;

    if output.boundary_reasons.len() != split_indices.len() {
      return Err("boundary_reasons length must match split_start_event_indices".to_owned());
    }

    if output
      .boundary_reasons
      .iter()
      .any(|reason| reason.is_time_gap())
    {
      return Err(
        "Large segment split output cannot use time gap as an internal boundary".to_owned(),
      );
    }

    Ok(Self::rebuild_split_segments(
      events,
      &output.split_start_event_indices,
      &output.boundary_reasons,
    )?)
  }

  fn rebuild_split_segments(
    events: &[Event],
    split_start_event_indices: &[u32],
    boundary_reasons: &[EventSegmentReason],
  ) -> Result<Vec<EventSegment>, String> {
    let split_indices = Self::validate_split_indices(events.len(), split_start_event_indices)?;
    if split_indices.len() != boundary_reasons.len() {
      return Err("boundary_reasons length must match split_start_event_indices".to_owned());
    }

    if split_indices.is_empty() {
      return Ok(vec![EventSegment::new(events.to_vec(), Vec::new())]);
    }

    let mut normalized = Vec::with_capacity(boundary_reasons.len() + 1);
    normalized.push(EventSegment::new(
      events[..split_indices[0]].to_vec(),
      Vec::new(),
    ));
    for (idx, split_index) in split_indices.iter().enumerate() {
      let end = split_indices.get(idx + 1).copied().unwrap_or(events.len());
      normalized.push(EventSegment::new(
        events[*split_index..end].to_vec(),
        vec![boundary_reasons[idx]],
      ));
    }
    Ok(normalized)
  }

  fn validate_split_indices(
    event_count: usize,
    split_indices: &[u32],
  ) -> Result<Vec<usize>, String> {
    let mut validated = Vec::with_capacity(split_indices.len());
    let mut previous = 0usize;

    for &split_index in split_indices {
      let split_index =
        usize::try_from(split_index).map_err(|_| "Split index conversion overflow".to_owned())?;
      if split_index == 0 {
        return Err("Split indices must not include 0".to_owned());
      }
      if split_index >= event_count {
        return Err("Split indices must be within the event range".to_owned());
      }
      if !validated.is_empty() && split_index <= previous {
        return Err("Split indices must be unique and strictly ascending".to_owned());
      }
      previous = split_index;
      validated.push(split_index);
    }

    Ok(validated)
  }

  fn time_gap_reason(gap: TimeDelta) -> EventSegmentReason {
    if gap > Self::HARD_TIME_GAP_THRESHOLD {
      EventSegmentReason::HardTimeGap
    } else {
      EventSegmentReason::TimeGap
    }
  }

  pub async fn segment(events: &[Event]) -> Result<Vec<EventSegment>, AppError> {
    let segments = Self::segment_by_time_gap(events)?;
    Self::segment_by_llm(segments).await
  }
}

#[cfg(test)]
mod tests {
  use chrono::{TimeZone, Utc};
  use plastmem_event::{Event, EventData, MessageEventData, MessageEventRole};

  use super::{EventSegmenter, LargeSegmentSplitOutput};
  use crate::EventSegmentReason;

  fn message_event(id: u128, minute: i64, role: MessageEventRole, content: &str) -> Event {
    Event::new(
      EventData::Message(MessageEventData {
        role,
        content: content.to_owned(),
      }),
      Utc
        .timestamp_opt(1_700_000_000 + minute * 60, 0)
        .single()
        .expect("valid timestamp"),
      Some(uuid::Uuid::from_u128(id)),
    )
  }

  #[test]
  fn time_gap_segmentation_builds_segments() {
    let events = vec![
      message_event(1, 0, MessageEventRole::User, "one"),
      message_event(2, 5, MessageEventRole::Assistant, "two"),
      message_event(3, 50, MessageEventRole::User, "three"),
    ];

    let segments = EventSegmenter::segment_by_time_gap(&events).expect("segments");
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0].events.len(), 2);
    assert_eq!(segments[1].events.len(), 1);
    assert_eq!(segments[1].reasons, &[EventSegmentReason::TimeGap]);
  }

  #[test]
  fn time_gap_segmentation_marks_hard_gaps() {
    let events = vec![
      message_event(1, 0, MessageEventRole::User, "one"),
      message_event(2, 181, MessageEventRole::Assistant, "two"),
    ];

    let segments = EventSegmenter::segment_by_time_gap(&events).expect("segments");

    assert_eq!(segments.len(), 2);
    assert_eq!(segments[1].reasons, &[EventSegmentReason::HardTimeGap]);
  }

  #[tokio::test]
  async fn hard_time_gap_blocks_small_segment_merge() {
    let mut previous = crate::EventSegment::new(
      vec![message_event(1, 0, MessageEventRole::User, "one")],
      Vec::new(),
    );
    let segment = crate::EventSegment::new(
      vec![message_event(2, 181, MessageEventRole::Assistant, "two")],
      vec![EventSegmentReason::HardTimeGap],
    );

    let result = EventSegmenter::check_small_segment(&mut previous, segment)
      .await
      .expect("small segment check");

    assert!(result.is_some());
    assert_eq!(previous.events.len(), 1);
  }

  #[test]
  fn validate_split_indices_rejects_invalid_shapes() {
    assert!(EventSegmenter::validate_split_indices(5, &[0]).is_err());
    assert!(EventSegmenter::validate_split_indices(5, &[5]).is_err());
    assert!(EventSegmenter::validate_split_indices(5, &[3, 2]).is_err());
    assert!(EventSegmenter::validate_split_indices(5, &[2, 2]).is_err());
  }

  #[test]
  fn rebuild_split_segments_assigns_boundary_reasons() {
    let events = vec![
      message_event(1, 0, MessageEventRole::User, "a"),
      message_event(2, 1, MessageEventRole::Assistant, "b"),
      message_event(3, 2, MessageEventRole::User, "c"),
      message_event(4, 3, MessageEventRole::Assistant, "d"),
    ];

    let segments =
      EventSegmenter::rebuild_split_segments(&events, &[2], &[EventSegmentReason::TopicShift])
        .expect("segments");

    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0].events.len(), 2);
    assert!(segments[0].reasons.is_empty());
    assert_eq!(segments[1].events.len(), 2);
    assert_eq!(segments[1].reasons, &[EventSegmentReason::TopicShift]);
  }

  #[test]
  fn replace_reason_overwrites_existing_boundary_reason() {
    let segment = crate::EventSegment::new(
      vec![
        message_event(1, 0, MessageEventRole::User, "a"),
        message_event(2, 1, MessageEventRole::Assistant, "b"),
      ],
      vec![EventSegmentReason::TopicShift],
    );

    let segment = segment.replace_reason(EventSegmentReason::IntentShift);

    assert_eq!(segment.reasons, &[EventSegmentReason::IntentShift]);
  }

  #[test]
  fn resolve_large_segment_split_rejects_time_gap_reason() {
    let events = vec![
      message_event(1, 0, MessageEventRole::User, "a"),
      message_event(2, 1, MessageEventRole::Assistant, "b"),
      message_event(3, 2, MessageEventRole::User, "c"),
    ];

    let result = EventSegmenter::resolve_large_segment_split(
      &events,
      LargeSegmentSplitOutput {
        split_start_event_indices: vec![1],
        boundary_reasons: vec![EventSegmentReason::TimeGap],
      },
    );

    assert!(result.is_err());
  }
}
