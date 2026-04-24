use std::collections::{BTreeMap, BTreeSet};

use plastmem_ai::{
  ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
  ChatCompletionRequestUserMessage, generate_object,
};
use plastmem_event::Event;
use plastmem_shared::AppError;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::{
  EventSegment, EventSegmentReason,
  prompt::{build_boundary_review_prompt, build_short_segment_merge_prompt},
};

use super::{MIN_SEGMENT_EVENTS, boundary::BoundaryCandidate, boundary::boundary_budget};

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
  StructuralCue,
  DetailElaboration,
  DirectResponse,
  Closing,
  Noise,
}

impl BoundaryLabel {
  fn is_boundary(self) -> bool {
    matches!(
      self,
      Self::TopicShift
        | Self::TopicIntro
        | Self::IntentShift
        | Self::ActivityShift
        | Self::StructuralCue
    )
  }

  fn to_segment_reason(self) -> EventSegmentReason {
    match self {
      Self::TopicShift | Self::TopicIntro => EventSegmentReason::TopicShift,
      Self::IntentShift => EventSegmentReason::IntentShift,
      Self::ActivityShift => EventSegmentReason::ActivityShift,
      Self::StructuralCue => EventSegmentReason::StructuralCue,
      Self::DetailElaboration | Self::DirectResponse | Self::Closing | Self::Noise => {
        EventSegmentReason::StructuralCue
      }
    }
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

pub(super) async fn review_candidates_with_llm(
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
        "Return JSON with keep_boundary_indices and decisions. Labels must be one of: topic_shift, topic_intro, intent_shift, activity_shift, structural_cue, detail_elaboration, direct_response, closing, noise.",
      )),
      ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage::from(
        build_boundary_review_prompt(events, &candidate_scores(&ranked), budget),
      )),
    ],
    "event_boundary_review_batch".to_owned(),
    Some("Review candidate event boundaries".to_owned()),
  )
  .await?;

  Ok(apply_boundary_review_output(&ranked, budget, output))
}

pub(super) async fn review_short_segments_with_llm(
  segments: &[EventSegment],
) -> Result<Vec<EventSegment>, AppError> {
  let short_indices = short_segment_indices(segments);
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

fn candidate_scores(candidates: &[BoundaryCandidate]) -> Vec<(usize, f32)> {
  candidates
    .iter()
    .map(|candidate| (candidate.index, candidate.score))
    .collect()
}

fn apply_boundary_review_output(
  candidates: &[BoundaryCandidate],
  budget: usize,
  output: BoundaryReviewOutput,
) -> Vec<BoundaryCandidate> {
  let candidate_by_index = candidates
    .iter()
    .map(|candidate| (candidate.index, candidate))
    .collect::<BTreeMap<_, _>>();
  let decisions = valid_boundary_decisions(output.decisions, &candidate_by_index);
  let mut kept = kept_boundaries(
    output.keep_boundary_indices,
    &candidate_by_index,
    &decisions,
  );

  if kept.is_empty() {
    kept = fallback_boundaries(&decisions);
  }

  kept.sort_by(|left, right| right.1.total_cmp(&left.1));
  kept.truncate(budget);
  kept.sort_by_key(|(index, _)| *index);

  kept
    .into_iter()
    .filter_map(|(index, _)| candidate_by_index.get(&index).copied())
    .map(|candidate| with_reviewed_reason(candidate, &decisions))
    .collect()
}

fn valid_boundary_decisions(
  decisions: Vec<BoundaryDecision>,
  candidate_by_index: &BTreeMap<usize, &BoundaryCandidate>,
) -> BTreeMap<usize, (BoundaryLabel, f32)> {
  decisions
    .into_iter()
    .filter_map(|decision| {
      let index = usize::try_from(decision.boundary_index).ok()?;
      candidate_by_index
        .contains_key(&index)
        .then_some((index, (decision.label, decision.confidence)))
    })
    .collect()
}

fn kept_boundaries(
  keep_boundary_indices: Vec<u32>,
  candidate_by_index: &BTreeMap<usize, &BoundaryCandidate>,
  decisions: &BTreeMap<usize, (BoundaryLabel, f32)>,
) -> Vec<(usize, f32)> {
  keep_boundary_indices
    .into_iter()
    .filter_map(|index| usize::try_from(index).ok())
    .filter(|index| candidate_by_index.contains_key(index))
    .filter(|index| {
      decisions
        .get(index)
        .is_none_or(|(label, confidence)| label.is_boundary() || *confidence < 0.55)
    })
    .map(|index| {
      let confidence = decisions
        .get(&index)
        .map_or(1.0, |(_, confidence)| *confidence);
      (index, confidence)
    })
    .collect()
}

fn fallback_boundaries(decisions: &BTreeMap<usize, (BoundaryLabel, f32)>) -> Vec<(usize, f32)> {
  decisions
    .iter()
    .filter(|(_, (label, confidence))| label.is_boundary() && *confidence >= 0.45)
    .map(|(index, (_, confidence))| (*index, *confidence))
    .collect()
}

fn with_reviewed_reason(
  candidate: &BoundaryCandidate,
  decisions: &BTreeMap<usize, (BoundaryLabel, f32)>,
) -> BoundaryCandidate {
  let mut candidate = candidate.clone();
  if let Some((label, _)) = decisions.get(&candidate.index) {
    candidate.reason = label.to_segment_reason();
  }
  candidate
}

fn short_segment_indices(segments: &[EventSegment]) -> Vec<usize> {
  segments
    .iter()
    .enumerate()
    .skip(1)
    .filter_map(|(index, segment)| {
      (segment.events.len() <= MIN_SEGMENT_EVENTS + 1).then_some(index)
    })
    .collect()
}

fn apply_short_segment_merge_decisions(
  segments: &[EventSegment],
  decisions: Vec<ShortSegmentMergeDecision>,
) -> Vec<EventSegment> {
  let merge_indices = merge_decisions_to_indices(decisions);
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

fn merge_decisions_to_indices(decisions: Vec<ShortSegmentMergeDecision>) -> BTreeSet<usize> {
  decisions
    .into_iter()
    .filter(|decision| {
      decision.merge_with_previous && decision.confidence >= 0.55 && decision.reason.is_merge()
    })
    .filter_map(|decision| usize::try_from(decision.segment_index).ok())
    .collect()
}

#[cfg(test)]
mod tests {
  use chrono::{TimeZone, Utc};
  use plastmem_event::{Event, EventData, MessageEventData, MessageEventRole};

  use super::*;

  fn candidate(index: usize, reason: EventSegmentReason) -> BoundaryCandidate {
    BoundaryCandidate {
      index,
      score: 0.8,
      reason,
    }
  }

  fn boundary_output(
    keep_boundary_indices: Vec<u32>,
    decisions: Vec<BoundaryDecision>,
  ) -> BoundaryReviewOutput {
    BoundaryReviewOutput {
      keep_boundary_indices,
      decisions,
    }
  }

  fn boundary_decision(
    boundary_index: u32,
    label: BoundaryLabel,
    confidence: f32,
  ) -> BoundaryDecision {
    BoundaryDecision {
      boundary_index,
      label,
      confidence,
    }
  }

  fn merge_decision(
    segment_index: u32,
    reason: ShortSegmentMergeReason,
    confidence: f32,
  ) -> ShortSegmentMergeDecision {
    ShortSegmentMergeDecision {
      segment_index,
      merge_with_previous: true,
      reason,
      confidence,
    }
  }

  fn segment(event_count: usize) -> EventSegment {
    let events = (0..event_count)
      .map(|index| {
        Event::new(
          EventData::Message(MessageEventData {
            role: MessageEventRole::User,
            content: format!("event {index}"),
          }),
          Utc
            .timestamp_opt(1_700_000_000 + index as i64 * 60, 0)
            .single()
            .expect("valid timestamp"),
          None,
        )
      })
      .collect();
    EventSegment::with_metadata(events, EventSegmentReason::TopicShift, 0.7, 0.5, 0.6)
  }

  #[test]
  fn boundary_review_keeps_explicit_valid_boundary() {
    let candidates = vec![candidate(4, EventSegmentReason::TopicShift)];
    let reviewed = apply_boundary_review_output(
      &candidates,
      1,
      boundary_output(
        vec![4],
        vec![boundary_decision(4, BoundaryLabel::IntentShift, 0.9)],
      ),
    );

    assert_eq!(reviewed.len(), 1);
    assert_eq!(reviewed[0].index, 4);
    assert_eq!(reviewed[0].reason, EventSegmentReason::IntentShift);
  }

  #[test]
  fn boundary_review_falls_back_to_confident_boundary_label() {
    let candidates = vec![candidate(4, EventSegmentReason::TopicShift)];
    let reviewed = apply_boundary_review_output(
      &candidates,
      1,
      boundary_output(
        Vec::new(),
        vec![boundary_decision(4, BoundaryLabel::ActivityShift, 0.5)],
      ),
    );

    assert_eq!(reviewed.len(), 1);
    assert_eq!(reviewed[0].reason, EventSegmentReason::ActivityShift);
  }

  #[test]
  fn boundary_review_rejects_high_confidence_non_boundary_label() {
    let candidates = vec![candidate(4, EventSegmentReason::TopicShift)];
    let reviewed = apply_boundary_review_output(
      &candidates,
      1,
      boundary_output(
        vec![4],
        vec![boundary_decision(4, BoundaryLabel::DirectResponse, 0.9)],
      ),
    );

    assert!(reviewed.is_empty());
  }

  #[test]
  fn short_segment_merge_ignores_low_confidence_or_separate_decisions() {
    let segments = vec![segment(6), segment(2), segment(2)];
    let merged = apply_short_segment_merge_decisions(
      &segments,
      vec![
        merge_decision(1, ShortSegmentMergeReason::SameTopicContinuation, 0.3),
        merge_decision(2, ShortSegmentMergeReason::SeparateTopic, 0.9),
      ],
    );

    assert_eq!(merged.len(), 3);
  }

  #[test]
  fn short_segment_merge_combines_valid_decision_with_previous_segment() {
    let segments = vec![segment(6), segment(2)];
    let merged = apply_short_segment_merge_decisions(
      &segments,
      vec![merge_decision(
        1,
        ShortSegmentMergeReason::DirectResponse,
        0.9,
      )],
    );

    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].events.len(), 8);
    assert_eq!(merged[0].boundary_after_confidence, 0.6);
  }
}
