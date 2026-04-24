use plastmem_ai::cosine_similarity;
use plastmem_event::Event;

use crate::EventSegmentReason;

use super::{
  MAX_REVIEW_CANDIDATES, MIN_SEGMENT_EVENTS, REVIEW_CONTEXT_EVENTS, SOFT_TIME_GAP,
  TARGET_EVENTS_PER_SEGMENT,
  embedding_stats::{mean_vector, segment_cohesion},
  time_gap_before,
};

#[derive(Debug, Clone)]
pub(super) struct BoundaryCandidate {
  pub(super) index: usize,
  pub(super) score: f32,
  pub(super) reason: EventSegmentReason,
}

pub(super) fn collect_candidates(
  events: &[Event],
  embeddings: &[Vec<f32>],
  prefix: &[Vec<f32>],
) -> Vec<BoundaryCandidate> {
  let mut candidates = (1..events.len())
    .filter(|&index| has_minimum_segment_size(events.len(), index))
    .map(|index| BoundaryCandidate {
      index,
      score: boundary_score(events, embeddings, prefix, index),
      reason: candidate_reason(events, index),
    })
    .collect::<Vec<_>>();

  candidates.sort_by(|left, right| right.score.total_cmp(&left.score));
  candidates.truncate(MAX_REVIEW_CANDIDATES.min(events.len().saturating_sub(1)));
  candidates.sort_by_key(|candidate| candidate.index);
  candidates
}

pub(super) fn boundary_budget(event_count: usize) -> usize {
  if event_count < TARGET_EVENTS_PER_SEGMENT * 2 {
    return 0;
  }

  let natural_budget = (event_count / TARGET_EVENTS_PER_SEGMENT).saturating_sub(1);
  let scaled_cap = (event_count / 24).clamp(1, 12);
  natural_budget.min(scaled_cap)
}

pub(super) fn reason_for(boundaries: &[BoundaryCandidate], index: usize) -> EventSegmentReason {
  boundaries
    .iter()
    .find(|candidate| candidate.index == index)
    .map(|candidate| candidate.reason)
    .unwrap_or(EventSegmentReason::InitialSegment)
}

pub(super) fn confidence_for(boundaries: &[BoundaryCandidate], index: usize) -> f32 {
  if index == 0 {
    return 1.0;
  }
  boundaries
    .iter()
    .find(|candidate| candidate.index == index)
    .map_or(1.0, |candidate| (candidate.score / 1.25).clamp(0.0, 1.0))
}

fn has_minimum_segment_size(event_count: usize, index: usize) -> bool {
  index >= MIN_SEGMENT_EVENTS && event_count - index >= MIN_SEGMENT_EVENTS
}

fn candidate_reason(events: &[Event], index: usize) -> EventSegmentReason {
  if time_gap_before(events, index) >= SOFT_TIME_GAP {
    EventSegmentReason::TimeGap
  } else {
    EventSegmentReason::TopicShift
  }
}

fn boundary_score(
  events: &[Event],
  embeddings: &[Vec<f32>],
  prefix: &[Vec<f32>],
  index: usize,
) -> f32 {
  let left_start = index.saturating_sub(REVIEW_CONTEXT_EVENTS);
  let right_end = (index + REVIEW_CONTEXT_EVENTS).min(events.len());
  let separation = 1.0
    - cosine_similarity(
      &mean_vector(prefix, left_start, index),
      &mean_vector(prefix, index, right_end),
    );
  let cohesion = (segment_cohesion(embeddings, prefix, left_start, index)
    + segment_cohesion(embeddings, prefix, index, right_end))
    * 0.5;
  let time_gap_bonus = if time_gap_before(events, index) >= SOFT_TIME_GAP {
    0.16
  } else {
    0.0
  };

  separation + 0.12 * cohesion + time_gap_bonus
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn boundary_budget_scales_with_partition_size() {
    assert_eq!(boundary_budget(23), 0);
    assert_eq!(boundary_budget(24), 1);
    assert_eq!(boundary_budget(37), 1);
    assert_eq!(boundary_budget(60), 2);
    assert_eq!(boundary_budget(120), 5);
    assert_eq!(boundary_budget(240), 10);
  }
}
