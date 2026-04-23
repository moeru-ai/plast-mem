use std::collections::BTreeSet;

use chrono::TimeDelta;
use plastmem_ai::{
  ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
  ChatCompletionRequestUserMessage, cosine_similarity, embed_many, generate_object,
};
use plastmem_event::{Event, EventDataToString};
use plastmem_shared::AppError;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::{EventSegment, EventSegmentReason};

pub struct EventSegmenter;

#[derive(Debug, Clone)]
struct BoundaryCandidate {
  index: usize,
  score: f32,
  reason: EventSegmentReason,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct BoundaryReviewOutput {
  keep_boundary_indices: Vec<u32>,
  decisions: Vec<BoundaryDecision>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct BoundaryDecision {
  boundary_index: u32,
  label: BoundaryLabel,
  confidence: f32,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ShortSegmentMergeReviewOutput {
  decisions: Vec<ShortSegmentMergeDecision>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ShortSegmentMergeDecision {
  segment_index: u32,
  merge_with_previous: bool,
  reason: ShortSegmentMergeReason,
  confidence: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum BoundaryLabel {
  TopicShift,
  TopicIntro,
  IntentShift,
  ActivityShift,
  DetailElaboration,
  DirectResponse,
  Closing,
  Noise,
}

impl BoundaryLabel {
  fn is_boundary(self) -> bool {
    matches!(
      self,
      Self::TopicShift | Self::TopicIntro | Self::IntentShift | Self::ActivityShift
    )
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ShortSegmentMergeReason {
  SameTopicContinuation,
  DetailElaboration,
  DirectResponse,
  ClosingOrFarewell,
  SeparateTopic,
  SeparateActivity,
  SeparateIntent,
}

impl ShortSegmentMergeReason {
  fn is_merge(self) -> bool {
    matches!(
      self,
      Self::SameTopicContinuation
        | Self::DetailElaboration
        | Self::DirectResponse
        | Self::ClosingOrFarewell
    )
  }
}

const SOFT_TIME_GAP: TimeDelta = TimeDelta::minutes(30);
const HARD_TIME_GAP: TimeDelta = TimeDelta::hours(3);
const MIN_SEGMENT_EVENTS: usize = 4;
const REVIEW_CONTEXT_EVENTS: usize = 5;
const MAX_REVIEW_CANDIDATES: usize = 40;
const TARGET_EVENTS_PER_SEGMENT: usize = 12;
const MAX_BOUNDARIES_PER_PARTITION: usize = 4;

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
      let is_partition_end = end == events.len()
        || events[end]
          .timestamp
          .signed_duration_since(events[end - 1].timestamp)
          > HARD_TIME_GAP;
      if !is_partition_end {
        continue;
      }

      let mut partition_segments =
        Self::segment_partition(&events[start..end], &embeddings[start..end]).await;
      if start > 0
        && let Some(first_segment) = partition_segments.first_mut()
      {
        first_segment.reason = EventSegmentReason::HardTimeGap;
      }
      segments.extend(partition_segments);
      start = end;
    }

    segments
  }

  async fn segment_partition(events: &[Event], embeddings: &[Vec<f32>]) -> Vec<EventSegment> {
    if events.len() <= 1 {
      return vec![EventSegment::with_metadata(
        events.to_vec(),
        EventSegmentReason::InitialSegment,
        1.0,
        1.0,
        1.0,
      )];
    }

    let prefix = build_prefix(embeddings);
    let candidates = collect_candidates(events, embeddings, &prefix);
    let reviewed = match review_candidates_with_llm(events, &candidates).await {
      Ok(reviewed) => reviewed,
      Err(err) => {
        tracing::warn!(error = %err, "Boundary review failed; using embedding candidates");
        candidates
          .into_iter()
          .take(boundary_budget(events.len()))
          .collect()
      }
    };

    let segments = build_segments(events, embeddings, &prefix, &reviewed);
    match review_short_segments_with_llm(&segments).await {
      Ok(segments) => segments,
      Err(err) => {
        tracing::warn!(error = %err, "Short segment merge review failed; keeping reviewed boundaries");
        segments
      }
    }
  }
}

fn collect_candidates(
  events: &[Event],
  embeddings: &[Vec<f32>],
  prefix: &[Vec<f32>],
) -> Vec<BoundaryCandidate> {
  let mut candidates = Vec::new();
  for index in 1..events.len() {
    if index < MIN_SEGMENT_EVENTS || events.len() - index < MIN_SEGMENT_EVENTS {
      continue;
    }
    candidates.push(BoundaryCandidate {
      index,
      score: boundary_score(events, embeddings, prefix, index),
      reason: if time_gap_before(events, index) >= SOFT_TIME_GAP {
        EventSegmentReason::TimeGap
      } else {
        EventSegmentReason::TopicShift
      },
    });
  }

  candidates.sort_by(|left, right| right.score.total_cmp(&left.score));
  candidates.truncate(MAX_REVIEW_CANDIDATES.min(events.len().saturating_sub(1)));
  candidates.sort_by_key(|candidate| candidate.index);
  candidates
}

async fn review_candidates_with_llm(
  events: &[Event],
  candidates: &[BoundaryCandidate],
) -> Result<Vec<BoundaryCandidate>, AppError> {
  let budget = boundary_budget(events.len());
  if candidates.is_empty() || budget == 0 {
    return Ok(Vec::new());
  }

  let mut ranked = candidates.to_vec();
  ranked.sort_by_key(|candidate| candidate.index);

  let output = generate_object::<BoundaryReviewOutput>(
    vec![
      ChatCompletionRequestMessage::System(ChatCompletionRequestSystemMessage::from(
        "Return JSON with keep_boundary_indices and decisions. Labels must be one of: topic_shift, topic_intro, intent_shift, activity_shift, detail_elaboration, direct_response, closing, noise.",
      )),
      ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage::from(
        build_review_prompt(events, &ranked),
      )),
    ],
    "event_boundary_review_batch".to_owned(),
    Some("Review candidate event boundaries".to_owned()),
  )
  .await?;

  let candidate_indices = ranked
    .iter()
    .map(|candidate| candidate.index)
    .collect::<BTreeSet<_>>();
  let labeled = output
    .decisions
    .into_iter()
    .filter_map(|decision| {
      let index = usize::try_from(decision.boundary_index).ok()?;
      candidate_indices
        .contains(&index)
        .then_some((index, decision.label, decision.confidence))
    })
    .collect::<Vec<_>>();

  let mut kept = output
    .keep_boundary_indices
    .into_iter()
    .filter_map(|index| usize::try_from(index).ok())
    .filter(|index| candidate_indices.contains(index))
    .filter(|index| {
      labeled
        .iter()
        .find(|(label_index, _, _)| label_index == index)
        .is_none_or(|(_, label, confidence)| label.is_boundary() || *confidence < 0.55)
    })
    .map(|index| {
      let confidence = labeled
        .iter()
        .find(|(label_index, _, _)| *label_index == index)
        .map_or(1.0, |(_, _, confidence)| *confidence);
      (index, confidence)
    })
    .collect::<Vec<_>>();

  if kept.is_empty() {
    kept = labeled
      .into_iter()
      .filter(|(_, label, confidence)| label.is_boundary() && *confidence >= 0.45)
      .map(|(index, _, confidence)| (index, confidence))
      .collect();
  }

  kept.sort_by(|left, right| right.1.total_cmp(&left.1));
  kept.truncate(budget);
  kept.sort_by_key(|(index, _)| *index);

  Ok(
    kept
      .into_iter()
      .filter_map(|(index, _)| {
        ranked
          .iter()
          .find(|candidate| candidate.index == index)
          .cloned()
      })
      .collect(),
  )
}

fn build_review_prompt(events: &[Event], candidates: &[BoundaryCandidate]) -> String {
  let mut prompt = format!(
    "Review candidate boundaries for a multilingual dialogue. Fill keep_boundary_indices with at most {} candidate indices that should be kept. Also return decisions for the kept indices, and optionally for nearby rejected candidates when useful. Nearby candidate indices may describe the same transition; choose the single best index, not all of them. Prefer fewer, larger event segments, but keep real pivots between unrelated subjects, activities, plans, stories, or intents. Use topic_shift/topic_intro/intent_shift/activity_shift for true boundaries. Use detail_elaboration/direct_response/closing/noise for continuations. Do not split follow-ups, clarifications, examples, greetings, or closing turns. It is valid to keep none.\n\n",
    boundary_budget(events.len())
  );

  for candidate in candidates {
    let left = candidate.index.saturating_sub(REVIEW_CONTEXT_EVENTS);
    let right = (candidate.index + REVIEW_CONTEXT_EVENTS).min(events.len());
    prompt.push_str(&format!(
      "Candidate boundary_index={} score={:.3}:\n",
      candidate.index, candidate.score
    ));
    for (offset, event) in events[left..right].iter().enumerate() {
      let index = left + offset;
      if index == candidate.index {
        prompt.push_str("  <BOUNDARY>\n");
      }
      prompt.push_str(&format!(
        "  [idx={index}] {} {}\n",
        event.timestamp.format("%Y-%m-%dT%H:%M:%SZ"),
        event.data.to_string_without_timestamp()
      ));
    }
    prompt.push('\n');
  }

  prompt
}

fn boundary_budget(event_count: usize) -> usize {
  if event_count < TARGET_EVENTS_PER_SEGMENT * 2 {
    return 0;
  }

  (event_count / TARGET_EVENTS_PER_SEGMENT)
    .saturating_sub(1)
    .min(MAX_BOUNDARIES_PER_PARTITION)
}

fn build_segments(
  events: &[Event],
  embeddings: &[Vec<f32>],
  prefix: &[Vec<f32>],
  boundaries: &[BoundaryCandidate],
) -> Vec<EventSegment> {
  let mut result = Vec::new();
  let mut start = 0usize;
  let mut points = boundaries
    .iter()
    .map(|candidate| candidate.index)
    .collect::<Vec<_>>();
  points.push(events.len());

  for end in points {
    if start >= end {
      continue;
    }
    let reason = boundaries
      .iter()
      .find(|candidate| candidate.index == start)
      .map(|candidate| candidate.reason)
      .unwrap_or(EventSegmentReason::InitialSegment);
    let score = segment_cohesion(embeddings, prefix, start, end);
    result.push(EventSegment::with_metadata(
      events[start..end].to_vec(),
      reason,
      score,
      confidence_for(boundaries, start),
      confidence_for(boundaries, end),
    ));
    start = end;
  }

  result
}

async fn review_short_segments_with_llm(
  segments: &[EventSegment],
) -> Result<Vec<EventSegment>, AppError> {
  let short_indices = segments
    .iter()
    .enumerate()
    .skip(1)
    .filter_map(|(index, segment)| {
      (segment.events.len() <= MIN_SEGMENT_EVENTS + 1).then_some(index)
    })
    .collect::<Vec<_>>();
  if short_indices.is_empty() {
    return Ok(segments.to_vec());
  }

  let output = generate_object::<ShortSegmentMergeReviewOutput>(
    vec![
      ChatCompletionRequestMessage::System(ChatCompletionRequestSystemMessage::from(
        "Return JSON with decisions. Reasons must be one of: same_topic_continuation, detail_elaboration, direct_response, closing_or_farewell, separate_topic, separate_activity, separate_intent.",
      )),
      ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage::from(
        build_short_segment_merge_prompt(segments, &short_indices),
      )),
    ],
    "event_short_segment_merge_review".to_owned(),
    Some("Review short event segments for semantic merge".to_owned()),
  )
  .await?;

  Ok(apply_short_segment_merge_decisions(
    segments,
    output.decisions,
  ))
}

fn build_short_segment_merge_prompt(segments: &[EventSegment], short_indices: &[usize]) -> String {
  let mut prompt = "Review short event segments in a multilingual dialogue. For each listed segment_index, decide whether the current short segment should merge with the immediately previous segment. Merge only when the short segment is a continuation, detail, direct response, greeting/closing, or small conversational tail of the previous event. Keep separate when it starts an independent topic, activity, intent, story, or plan. Use semantic relation across languages; do not rely on English keywords. Return one decision for every listed segment_index.\n\n".to_owned();

  for &index in short_indices {
    prompt.push_str(&format!("segment_index={index}\nprevious_segment_tail:\n"));
    let previous = &segments[index - 1].events;
    let previous_start = previous.len().saturating_sub(REVIEW_CONTEXT_EVENTS + 3);
    append_events(&mut prompt, &previous[previous_start..]);
    prompt.push_str("current_short_segment:\n");
    append_events(&mut prompt, &segments[index].events);
    prompt.push('\n');
  }

  prompt
}

fn append_events(prompt: &mut String, events: &[Event]) {
  for event in events {
    prompt.push_str(&format!(
      "  {} {}\n",
      event.timestamp.format("%Y-%m-%dT%H:%M:%SZ"),
      event.data.to_string_without_timestamp()
    ));
  }
}

fn apply_short_segment_merge_decisions(
  segments: &[EventSegment],
  decisions: Vec<ShortSegmentMergeDecision>,
) -> Vec<EventSegment> {
  let merge_indices = decisions
    .into_iter()
    .filter(|decision| {
      decision.merge_with_previous && decision.confidence >= 0.55 && decision.reason.is_merge()
    })
    .filter_map(|decision| usize::try_from(decision.segment_index).ok())
    .collect::<BTreeSet<_>>();

  let mut merged: Vec<EventSegment> = Vec::new();
  for (index, segment) in segments.iter().cloned().enumerate() {
    if index > 0
      && merge_indices.contains(&index)
      && let Some(previous) = merged.last_mut()
    {
      previous.events.extend(segment.events);
      previous.boundary_after_confidence = segment.boundary_after_confidence;
      previous.score = previous.score.min(segment.score);
    } else {
      merged.push(segment);
    }
  }
  merged
}

fn build_prefix(embeddings: &[Vec<f32>]) -> Vec<Vec<f32>> {
  let dims = embeddings.first().map_or(0, Vec::len);
  let mut prefix = vec![vec![0.0; dims]];
  for embedding in embeddings {
    let mut next = prefix.last().cloned().unwrap_or_else(|| vec![0.0; dims]);
    for (value, current) in next.iter_mut().zip(embedding) {
      *value += current;
    }
    prefix.push(next);
  }
  prefix
}

fn boundary_score(
  events: &[Event],
  embeddings: &[Vec<f32>],
  prefix: &[Vec<f32>],
  index: usize,
) -> f32 {
  let left_start = index.saturating_sub(REVIEW_CONTEXT_EVENTS);
  let right_end = (index + REVIEW_CONTEXT_EVENTS).min(events.len());
  let left = mean_vector(prefix, left_start, index);
  let right = mean_vector(prefix, index, right_end);
  let separation = 1.0 - cosine_similarity(&left, &right);
  let cohesion = (segment_cohesion(embeddings, prefix, left_start, index)
    + segment_cohesion(embeddings, prefix, index, right_end))
    * 0.5;
  let mut score = separation + 0.12 * cohesion;

  if time_gap_before(events, index) >= SOFT_TIME_GAP {
    score += 0.16;
  }
  score
}

fn mean_vector(prefix: &[Vec<f32>], start: usize, end: usize) -> Vec<f32> {
  if start >= end || prefix.is_empty() {
    return Vec::new();
  }

  let count = (end - start) as f32;
  let mut mean = prefix[end]
    .iter()
    .zip(&prefix[start])
    .map(|(right, left)| (right - left) / count)
    .collect::<Vec<_>>();
  normalize(&mut mean);
  mean
}

fn normalize(vector: &mut [f32]) {
  let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
  if norm > f32::EPSILON {
    for value in vector {
      *value /= norm;
    }
  }
}

fn segment_cohesion(embeddings: &[Vec<f32>], prefix: &[Vec<f32>], start: usize, end: usize) -> f32 {
  if end <= start + 1 {
    return 1.0;
  }

  let mean = mean_vector(prefix, start, end);
  let total = embeddings[start..end]
    .iter()
    .map(|embedding| cosine_similarity(embedding, &mean))
    .sum::<f32>();
  (total / (end - start) as f32).clamp(-1.0, 1.0)
}

fn time_gap_before(events: &[Event], index: usize) -> TimeDelta {
  if index == 0 || index >= events.len() {
    return TimeDelta::zero();
  }
  events[index]
    .timestamp
    .signed_duration_since(events[index - 1].timestamp)
}

fn confidence_for(boundaries: &[BoundaryCandidate], index: usize) -> f32 {
  if index == 0 || boundaries.iter().all(|candidate| candidate.index != index) {
    return 1.0;
  }
  boundaries
    .iter()
    .find(|candidate| candidate.index == index)
    .map_or(0.35, |candidate| (candidate.score / 1.25).clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
  use chrono::{TimeZone, Utc};
  use plastmem_event::{Event, EventData, MessageEventData, MessageEventRole};

  use super::*;

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

  #[test]
  fn boundary_budget_scales_with_partition_size() {
    assert_eq!(boundary_budget(23), 0);
    assert_eq!(boundary_budget(24), 1);
    assert_eq!(boundary_budget(37), 2);
    assert_eq!(boundary_budget(60), MAX_BOUNDARIES_PER_PARTITION);
  }
}
