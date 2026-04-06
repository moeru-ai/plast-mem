use std::collections::{BTreeSet, HashSet};

use anyhow::anyhow;
use apalis::prelude::{Data, TaskSink};
use apalis_postgres::PostgresStorage;
use chrono::{DateTime, TimeDelta, Utc};
use fsrs::{DEFAULT_PARAMETERS, FSRS};
use plastmem_ai::{
  ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
  ChatCompletionRequestUserMessage, embed, generate_object,
};
use plastmem_core::{
  ConversationMessageRecord, MessageQueue, SEGMENTATION_WINDOW_BASE, SegmentationBoundaryContext,
  get_segmentation_state, list_messages,
};
use plastmem_entities::{episode_span, episodic_memory, segmentation_state};
use plastmem_shared::{APP_ENV, AppError, Message};
use schemars::JsonSchema;
use sea_orm::{
  ActiveModelTrait, DatabaseConnection, EntityTrait, IntoActiveModel, QuerySelect, Set,
  TransactionTrait,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{MemoryReviewJob, PredictCalibrateJob};

const FLASHBULB_SURPRISE_THRESHOLD: f32 = 0.85;
const DESIRED_RETENTION: f32 = 0.9;
const SURPRISE_BOOST_FACTOR: f32 = 0.5;
const MAX_BOUNDARY_CONTEXT_LINES: usize = 3;

const LOCAL_BOUNDARY_REFINER_SYSTEM_PROMPT: &str = r#"
You are validating one candidate event boundary in a conversation.
Return only JSON that matches the schema.

Decide whether the candidate boundary is a real episodic split.

Use these principles:
- Favor topic shift, intent shift, temporal gap, and surprise discontinuity.
- Do NOT split ordinary back-and-forth turns, acknowledgements, answers, or follow-up questions when they are still about the same local exchange.
- Keep short question-answer exchanges together unless there is a clear new topic, a clear new activity, or a strong temporal/session break.
- Approve clear same-session topic changes when the new turns open a materially different subject, activity, or event, even without a temporal gap.
- A boundary must land on the first message of the new segment.
- `boundary_reason` must be exactly one of:
  `topic_shift`, `intent_shift`, `surprise_shift`, `temporal_gap`, `session_break`
- `surprise_level` must be exactly one of:
  `low`, `high`, `extremely_high`
- `surprise_level` measures how abruptly the new segment begins relative to the prior segment.
- If the candidate is weak, reject it.
"#;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventSegmentationJob {
  pub conversation_id: Uuid,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum SurpriseLevel {
  Low,
  High,
  ExtremelyHigh,
}

impl SurpriseLevel {
  const fn to_signal(self) -> f32 {
    match self {
      Self::Low => 0.2,
      Self::High => 0.6,
      Self::ExtremelyHigh => 0.9,
    }
  }

  const fn as_str(self) -> &'static str {
    match self {
      Self::Low => "low",
      Self::High => "high",
      Self::ExtremelyHigh => "extremely_high",
    }
  }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum BoundaryReason {
  TopicShift,
  IntentShift,
  SurpriseShift,
  TemporalGap,
  SessionBreak,
}

impl BoundaryReason {
  const fn as_str(self) -> &'static str {
    match self {
      Self::TopicShift => "topic_shift",
      Self::IntentShift => "intent_shift",
      Self::SurpriseShift => "surprise_shift",
      Self::TemporalGap => "temporal_gap",
      Self::SessionBreak => "session_break",
    }
  }
}

#[derive(Debug, Deserialize, JsonSchema)]
struct LocalBoundaryRefinerOutput {
  is_boundary: bool,
  boundary_reason: BoundaryReason,
  surprise_level: SurpriseLevel,
}

#[derive(Debug, Clone)]
struct AnalysisUnit {
  start_seq: i64,
  end_seq: i64,
  messages: Vec<ConversationMessageRecord>,
  joined_content: String,
  token_set: BTreeSet<String>,
  start_at: DateTime<Utc>,
  end_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct CandidateBoundary {
  next_unit_index: usize,
  score: f32,
  time_gap: f32,
  cue_phrase: f32,
  semantic_drop: f32,
  online_surprise_prior: f32,
  micro_exchange_penalty: f32,
}

#[derive(Debug, Clone)]
struct RefinedBoundary {
  next_unit_index: usize,
  boundary_reason: BoundaryReason,
  surprise_level: SurpriseLevel,
}

#[derive(Debug, Clone)]
struct ClosedSpan {
  start_seq: i64,
  end_seq: i64,
  messages: Vec<Message>,
  boundary_reason: BoundaryReason,
  surprise_level: SurpriseLevel,
}

#[derive(Debug, Clone)]
struct CreatedEpisode {
  id: Uuid,
  surprise: f32,
}

pub async fn process_event_segmentation(
  job: EventSegmentationJob,
  db: Data<DatabaseConnection>,
  segmentation_storage: Data<PostgresStorage<EventSegmentationJob>>,
  review_storage: Data<PostgresStorage<MemoryReviewJob>>,
  semantic_storage: Data<PostgresStorage<PredictCalibrateJob>>,
) -> Result<(), AppError> {
  let db = &*db;
  let state = get_segmentation_state(job.conversation_id, db).await?;
  let Some(in_progress_until_seq) = state.in_progress_until_seq else {
    return Ok(());
  };

  let read_start_seq = state.open_tail_start_seq.unwrap_or(state.next_unsegmented_seq);
  if read_start_seq > in_progress_until_seq {
    finalize_empty_pass(job.conversation_id, db).await?;
    return Ok(());
  }

  let records = list_messages(job.conversation_id, read_start_seq, in_progress_until_seq, db).await?;
  if records.is_empty() {
    finalize_empty_pass(job.conversation_id, db).await?;
    return Ok(());
  }

  let analysis_units = build_analysis_units(&records);
  let candidates = score_candidate_boundaries(&analysis_units, state.last_closed_boundary_context.as_ref());
  let refined_boundaries = refine_candidate_boundaries(
    &analysis_units,
    &candidates,
    state.last_closed_boundary_context.as_ref(),
  )
  .await?;
  let closed_spans = close_segments(
    &analysis_units,
    &refined_boundaries,
    state.open_tail_start_seq.is_some(),
    state.eof_seen,
  );

  enqueue_pending_reviews(job.conversation_id, &to_messages(&records), db, &review_storage).await?;
  let created = persist_closed_spans(
    job.conversation_id,
    in_progress_until_seq,
    state.eof_seen,
    &analysis_units,
    &closed_spans,
    db,
  )
  .await?;
  enqueue_predict_calibrate_jobs(job.conversation_id, &created, &semantic_storage).await?;
  enqueue_follow_up_if_needed(job.conversation_id, db, &segmentation_storage).await?;

  Ok(())
}

fn build_analysis_units(records: &[ConversationMessageRecord]) -> Vec<AnalysisUnit> {
  let mut units = Vec::new();

  for record in records {
    let should_merge = units.last().is_some_and(|unit| should_merge_into_unit(unit, record));
    if should_merge {
      let unit = units.last_mut().expect("unit exists");
      unit.end_seq = record.seq;
      unit.end_at = record.message.timestamp;
      unit.messages.push(record.clone());
      if !unit.joined_content.is_empty() {
        unit.joined_content.push('\n');
      }
      unit.joined_content.push_str(&format_message_line(record));
      unit.token_set.extend(tokenize(&record.message.content));
      continue;
    }

    units.push(AnalysisUnit {
      start_seq: record.seq,
      end_seq: record.seq,
      messages: vec![record.clone()],
      joined_content: format_message_line(record),
      token_set: tokenize(&record.message.content),
      start_at: record.message.timestamp,
      end_at: record.message.timestamp,
    });
  }

  units
}

fn should_merge_into_unit(unit: &AnalysisUnit, next: &ConversationMessageRecord) -> bool {
  let Some(last_message) = unit.messages.last() else {
    return false;
  };

  if last_message.message.role != next.message.role {
    return false;
  }

  if next.message.timestamp - last_message.message.timestamp > TimeDelta::minutes(5) {
    return false;
  }

  let combined_chars = unit
    .messages
    .iter()
    .map(|message| message.message.content.len())
    .sum::<usize>()
    + next.message.content.len();
  combined_chars <= 240 && next.message.content.len() <= 160
}

fn score_candidate_boundaries(
  units: &[AnalysisUnit],
  boundary_context: Option<&SegmentationBoundaryContext>,
) -> Vec<CandidateBoundary> {
  let mut candidates = Vec::new();

  for next_unit_index in 1..units.len() {
    let previous = &units[next_unit_index - 1];
    let current = &units[next_unit_index];
    let gap_minutes = (current.start_at - previous.end_at).num_minutes();
    let time_gap = match gap_minutes {
      n if n >= 180 => 1.0,
      n if n >= 60 => 0.8,
      n if n >= 30 => 0.55,
      n if n >= 10 => 0.25,
      _ => 0.0,
    };

    let cue_phrase = structural_boundary_cue(previous, current);

    let micro_exchange_penalty =
      micro_exchange_penalty(units, next_unit_index, time_gap, cue_phrase);
    let adjacent_drop = semantic_drop_score(previous, current);
    let window_drop = window_semantic_drop(units, next_unit_index);
    let semantic_drop =
      (adjacent_drop.max(window_drop) - micro_exchange_penalty * 0.35).clamp(0.0, 1.0);
    let online_surprise_prior =
      online_surprise_prior(previous, current, boundary_context, time_gap, cue_phrase, semantic_drop);
    let score =
      (time_gap + cue_phrase + semantic_drop + online_surprise_prior - micro_exchange_penalty).max(0.0);

    let strong_context_free_candidate =
      semantic_drop >= 0.76 && online_surprise_prior >= 0.18 && micro_exchange_penalty < 0.25;
    if score >= 1.45 || time_gap >= 0.8 || cue_phrase >= 0.7 || strong_context_free_candidate {
      candidates.push(CandidateBoundary {
        next_unit_index,
        score,
        time_gap,
        cue_phrase,
        semantic_drop,
        online_surprise_prior,
        micro_exchange_penalty,
      });
    }
  }

  suppress_dense_same_session_candidates(candidates)
}

fn suppress_dense_same_session_candidates(
  candidates: Vec<CandidateBoundary>,
) -> Vec<CandidateBoundary> {
  let mut filtered = Vec::new();
  let mut index = 0usize;

  while index < candidates.len() {
    let candidate = &candidates[index];
    if candidate.time_gap > 0.0 || candidate.cue_phrase > 0.0 {
      filtered.push(candidate.clone());
      index += 1;
      continue;
    }

    let mut best = candidate.clone();
    let mut end = index + 1;
    while end < candidates.len() {
      let next = &candidates[end];
      let same_session_cluster = next.time_gap == 0.0
        && next.cue_phrase == 0.0
        && next.next_unit_index.saturating_sub(candidates[end - 1].next_unit_index) <= 2;
      if !same_session_cluster {
        break;
      }
      if next.score >= best.score {
        best = next.clone();
      }
      end += 1;
    }

    filtered.push(best);
    index = end;
  }

  filtered
}

fn semantic_drop_score(previous: &AnalysisUnit, current: &AnalysisUnit) -> f32 {
  let token_drop = lexical_set_distance(&previous.token_set, &current.token_set);
  let char_drop = 1.0 - char_ngram_overlap(&unit_surface_text(previous), &unit_surface_text(current));
  (0.6 * token_drop + 0.4 * char_drop).clamp(0.0, 1.0)
}

fn lexical_set_distance(previous: &BTreeSet<String>, current: &BTreeSet<String>) -> f32 {
  if previous.is_empty() || current.is_empty() {
    return 0.1;
  }

  let overlap = previous.iter().filter(|token| current.contains(*token)).count() as f32;
  let union = previous.union(current).count() as f32;
  (1.0 - overlap / union).clamp(0.0, 1.0)
}

fn window_semantic_drop(units: &[AnalysisUnit], next_unit_index: usize) -> f32 {
  let previous_start = next_unit_index.saturating_sub(3);
  let current_end = (next_unit_index + 2).min(units.len());
  let previous_window = &units[previous_start..next_unit_index];
  let current_window = &units[next_unit_index..current_end];
  if previous_window.is_empty() || current_window.is_empty() {
    return 0.0;
  }

  let previous_tokens = aggregate_token_set(previous_window);
  let current_tokens = aggregate_token_set(current_window);
  let token_drop = lexical_set_distance(&previous_tokens, &current_tokens);
  let char_drop = 1.0
    - char_ngram_overlap(
      &aggregate_window_text(previous_window),
      &aggregate_window_text(current_window),
    );
  (0.55 * token_drop + 0.45 * char_drop).clamp(0.0, 1.0)
}

fn online_surprise_prior(
  previous: &AnalysisUnit,
  current: &AnalysisUnit,
  boundary_context: Option<&SegmentationBoundaryContext>,
  time_gap: f32,
  cue_phrase: f32,
  semantic_drop: f32,
) -> f32 {
  let context_tokens = boundary_context
    .map(|context| {
      tokenize(&context.anchor_topic)
        .into_iter()
        .chain(
          context
            .anchor_entities
            .iter()
            .flat_map(|entity| tokenize(entity).into_iter()),
        )
        .collect::<HashSet<_>>()
    })
    .unwrap_or_default();

  let context_novelty = if context_tokens.is_empty() {
    0.0
  } else {
    let overlap = current
      .token_set
      .iter()
      .filter(|token| context_tokens.contains(*token))
      .count() as f32;
    let current_len = current.token_set.len().max(1) as f32;
    (1.0 - overlap / current_len).clamp(0.0, 1.0)
  };

  let speaker_shift = if previous
    .messages
    .last()
    .zip(current.messages.first())
    .is_some_and(|(prev, next)| prev.message.role != next.message.role)
  {
    0.05
  } else {
    0.0
  };

  (0.4 * time_gap + 0.25 * cue_phrase + 0.15 * semantic_drop + 0.15 * context_novelty + speaker_shift)
    .clamp(0.0, 1.0)
}

fn micro_exchange_penalty(
  units: &[AnalysisUnit],
  next_unit_index: usize,
  time_gap: f32,
  cue_phrase: f32,
) -> f32 {
  let previous = &units[next_unit_index - 1];
  let current = &units[next_unit_index];

  if time_gap > 0.0 || cue_phrase > 0.0 {
    return 0.0;
  }

  let Some(previous_last) = previous.messages.last() else {
    return 0.0;
  };
  let Some(current_first) = current.messages.first() else {
    return 0.0;
  };

  if previous_last.message.role == current_first.message.role {
    return 0.0;
  }

  let previous_text = previous_last.message.content.trim();
  let current_text = current_first.message.content.trim();
  let previous_short = previous_text.len() <= 220;
  let current_short = current_text.len() <= 220;
  if !previous_short || !current_short {
    return 0.0;
  }

  let previous_tokens = &previous.token_set;
  let current_tokens = &current.token_set;
  let shared_tokens = previous_tokens
    .iter()
    .filter(|token| current_tokens.contains(*token))
    .count();
  let question_answer_shape =
    ends_with_question_marker(previous_text) || ends_with_question_marker(current_text);
  let char_overlap = char_ngram_overlap(previous_text, current_text);
  let compact_turn_pair = previous_text.len() <= 140 && current_text.len() <= 140;

  if question_answer_shape {
    return 0.6;
  }

  if shared_tokens >= 1 || (compact_turn_pair && char_overlap >= 0.18) {
    return 0.45;
  }

  if forms_recent_question_answer_exchange(units, next_unit_index, compact_turn_pair) {
    return 0.35;
  }

  if compact_turn_pair && previous.messages.len() == 1 && current.messages.len() == 1 {
    return 0.1;
  }

  0.0
}

fn forms_recent_question_answer_exchange(
  units: &[AnalysisUnit],
  next_unit_index: usize,
  compact_turn_pair: bool,
) -> bool {
  if next_unit_index < 2 || !compact_turn_pair {
    return false;
  }

  let anchor = &units[next_unit_index - 2];
  let previous = &units[next_unit_index - 1];
  let current = &units[next_unit_index];
  let Some(anchor_last) = anchor.messages.last() else {
    return false;
  };
  let Some(previous_last) = previous.messages.last() else {
    return false;
  };
  let Some(current_first) = current.messages.first() else {
    return false;
  };

  if !ends_with_question_marker(&anchor_last.message.content) {
    return false;
  }

  let alternating_roles = anchor_last.message.role == current_first.message.role
    && anchor_last.message.role != previous_last.message.role;
  let compact_triads = anchor_last.message.content.len() <= 220
    && previous_last.message.content.len() <= 220
    && current_first.message.content.len() <= 220;

  alternating_roles && compact_triads
}

async fn refine_candidate_boundaries(
  units: &[AnalysisUnit],
  candidates: &[CandidateBoundary],
  boundary_context: Option<&SegmentationBoundaryContext>,
) -> Result<Vec<RefinedBoundary>, AppError> {
  let mut refined = Vec::new();

  for candidate in candidates {
    let output = match request_boundary_refinement(units, candidate, boundary_context).await {
      Ok(output) => output,
      Err(error) => {
        tracing::warn!(
          candidate_index = candidate.next_unit_index,
          score = candidate.score,
          "Boundary refinement failed, falling back to deterministic decision: {error}"
        );
        fallback_boundary_refinement(candidate)
      }
    };

    if output.is_boundary {
      refined.push(RefinedBoundary {
        next_unit_index: candidate.next_unit_index,
        boundary_reason: output.boundary_reason,
        surprise_level: output.surprise_level,
      });
    }
  }

  Ok(refined)
}

async fn request_boundary_refinement(
  units: &[AnalysisUnit],
  candidate: &CandidateBoundary,
  boundary_context: Option<&SegmentationBoundaryContext>,
) -> Result<LocalBoundaryRefinerOutput, AppError> {
  let start = candidate.next_unit_index.saturating_sub(2);
  let end = (candidate.next_unit_index + 2).min(units.len().saturating_sub(1));
  let mut lines = Vec::new();
  for (index, unit) in units.iter().enumerate().take(end + 1).skip(start) {
    let marker = if index == candidate.next_unit_index {
      ">>> candidate new segment starts here"
    } else {
      "context"
    };
    lines.push(format!(
      "[unit {index}] {marker}\n{}",
      unit.joined_content
    ));
  }

  let boundary_context = boundary_context.map_or_else(
    || "none".to_owned(),
    |context| serde_json::to_string(context).unwrap_or_else(|_| "none".to_owned()),
  );

  let user = format!(
    "Boundary context: {boundary_context}\n\
     Candidate score breakdown:\n\
     - time_gap: {:.2}\n\
     - cue_phrase: {:.2}\n\
     - semantic_drop: {:.2}\n\
     - online_surprise_prior: {:.2}\n\
     - micro_exchange_penalty: {:.2}\n\n\
     Local units:\n{}",
    candidate.time_gap,
    candidate.cue_phrase,
    candidate.semantic_drop,
    candidate.online_surprise_prior,
    candidate.micro_exchange_penalty,
    lines.join("\n\n")
  );

  generate_object::<LocalBoundaryRefinerOutput>(
    vec![
      ChatCompletionRequestMessage::System(ChatCompletionRequestSystemMessage::from(
        LOCAL_BOUNDARY_REFINER_SYSTEM_PROMPT.trim(),
      )),
      ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage::from(user)),
    ],
    "local_boundary_refiner_output".to_owned(),
    Some("Event boundary validation result".to_owned()),
  )
  .await
}

fn fallback_boundary_refinement(candidate: &CandidateBoundary) -> LocalBoundaryRefinerOutput {
  let is_boundary = candidate.score >= 1.85
    || candidate.time_gap >= 0.8
    || (candidate.cue_phrase >= 0.7 && candidate.micro_exchange_penalty < 0.25);
  let boundary_reason = if candidate.time_gap >= 0.8 {
    BoundaryReason::TemporalGap
  } else if candidate.cue_phrase >= 0.7 {
    BoundaryReason::TopicShift
  } else if candidate.online_surprise_prior >= 0.6 {
    BoundaryReason::SurpriseShift
  } else {
    BoundaryReason::IntentShift
  };
  let surprise_level = if candidate.score >= 2.1 || candidate.time_gap >= 1.0 {
    SurpriseLevel::ExtremelyHigh
  } else if candidate.score >= 1.5 {
    SurpriseLevel::High
  } else {
    SurpriseLevel::Low
  };

  LocalBoundaryRefinerOutput {
    is_boundary,
    boundary_reason,
    surprise_level,
  }
}

fn close_segments(
  units: &[AnalysisUnit],
  refined_boundaries: &[RefinedBoundary],
  has_open_tail: bool,
  eof_seen: bool,
) -> Vec<ClosedSpan> {
  if units.is_empty() {
    return Vec::new();
  }

  let mut boundaries = refined_boundaries.to_vec();
  boundaries.sort_by_key(|boundary| boundary.next_unit_index);

  let mut closed = Vec::new();
  let mut segment_start_index = 0usize;
  let mut segment_start_boundary: Option<&RefinedBoundary> = None;

  for boundary in &boundaries {
    let end_index = boundary.next_unit_index.saturating_sub(1);
    if end_index >= segment_start_index {
      closed.push(build_closed_span(
        units,
        segment_start_index,
        end_index,
        segment_start_boundary,
        has_open_tail && segment_start_index == 0,
      ));
    }
    segment_start_index = boundary.next_unit_index;
    segment_start_boundary = Some(boundary);
  }

  if eof_seen && segment_start_index < units.len() {
    closed.push(build_closed_span(
      units,
      segment_start_index,
      units.len() - 1,
      segment_start_boundary,
      has_open_tail && segment_start_index == 0,
    ));
  }

  normalize_closed_spans(closed)
}

fn build_closed_span(
  units: &[AnalysisUnit],
  start_index: usize,
  end_index: usize,
  start_boundary: Option<&RefinedBoundary>,
  continuing_open_tail: bool,
) -> ClosedSpan {
  let messages = units[start_index..=end_index]
    .iter()
    .flat_map(|unit| unit.messages.iter().map(|record| record.message.clone()))
    .collect::<Vec<_>>();

  let (boundary_reason, surprise_level) = match start_boundary {
    Some(boundary) => (boundary.boundary_reason, boundary.surprise_level),
    None if continuing_open_tail => (BoundaryReason::TopicShift, SurpriseLevel::Low),
    None => (BoundaryReason::SessionBreak, SurpriseLevel::Low),
  };

  ClosedSpan {
    start_seq: units[start_index].start_seq,
    end_seq: units[end_index].end_seq,
    messages,
    boundary_reason,
    surprise_level,
  }
}

fn normalize_closed_spans(mut spans: Vec<ClosedSpan>) -> Vec<ClosedSpan> {
  let mut index = 0usize;
  while index < spans.len() {
    if !should_merge_short_span(&spans, index) {
      index += 1;
      continue;
    }

    let can_merge_into_previous = index > 0;
    let can_merge_into_next = index + 1 < spans.len() && !is_strong_start_boundary(&spans[index + 1]);

    let previous_affinity = if can_merge_into_previous {
      span_affinity(&spans[index - 1], &spans[index])
    } else {
      -1.0
    };
    let next_affinity = if can_merge_into_next {
      span_affinity(&spans[index], &spans[index + 1])
    } else {
      -1.0
    };
    let merge_threshold = merge_affinity_threshold(&spans[index]);

    let merged = if previous_affinity >= next_affinity && previous_affinity >= merge_threshold {
      let messages = spans[index].messages.clone();
      let end_seq = spans[index].end_seq;
      spans[index - 1].end_seq = end_seq;
      spans[index - 1].messages.extend(messages);
      spans.remove(index);
      true
    } else if next_affinity >= merge_threshold {
      let current = spans.remove(index);
      spans[index].start_seq = current.start_seq;
      spans[index].boundary_reason = current.boundary_reason;
      spans[index].surprise_level = current.surprise_level;
      let mut messages = current.messages;
      messages.extend(spans[index].messages.clone());
      spans[index].messages = messages;
      true
    } else {
      false
    };

    if !merged {
      index += 1;
    }
  }

  spans
}

fn should_merge_short_span(spans: &[ClosedSpan], index: usize) -> bool {
  let span = &spans[index];
  span.messages.len() <= 2
    && matches!(span.boundary_reason, BoundaryReason::TopicShift | BoundaryReason::IntentShift)
    && span.surprise_level != SurpriseLevel::ExtremelyHigh
}

fn merge_affinity_threshold(span: &ClosedSpan) -> f32 {
  match (span.messages.len(), span.surprise_level) {
    (0, _) => 1.0,
    (1, SurpriseLevel::Low) => 0.08,
    (1, SurpriseLevel::High) => 0.15,
    (2, SurpriseLevel::Low) => 0.14,
    (2, SurpriseLevel::High) => 0.20,
    _ => 1.0,
  }
}

fn is_strong_start_boundary(span: &ClosedSpan) -> bool {
  matches!(
    span.boundary_reason,
    BoundaryReason::TemporalGap | BoundaryReason::SessionBreak | BoundaryReason::SurpriseShift
  ) || span.surprise_level == SurpriseLevel::ExtremelyHigh
}

fn span_affinity(left: &ClosedSpan, right: &ClosedSpan) -> f32 {
  let left_text = span_surface_text(left);
  let right_text = span_surface_text(right);
  let char_affinity = char_ngram_overlap(&left_text, &right_text);
  let left_tokens = span_token_set(left);
  let right_tokens = span_token_set(right);
  let shared_tokens = left_tokens
    .iter()
    .filter(|token| right_tokens.contains(*token))
    .count() as f32;
  let token_affinity = if left_tokens.is_empty() || right_tokens.is_empty() {
    0.0
  } else {
    1.0 - lexical_set_distance(&left_tokens, &right_tokens)
  };
  let shared_token_boost = (shared_tokens.min(3.0) / 3.0) * 0.12;

  (0.5 * char_affinity + 0.38 * token_affinity + shared_token_boost).clamp(0.0, 1.0)
}

async fn persist_closed_spans(
  conversation_id: Uuid,
  claimed_until_seq: i64,
  eof_seen: bool,
  units: &[AnalysisUnit],
  closed_spans: &[ClosedSpan],
  db: &DatabaseConnection,
) -> Result<Vec<CreatedEpisode>, AppError> {
  let txn = db.begin().await?;
  let Some(state_model) = segmentation_state::Entity::find_by_id(conversation_id)
    .lock_exclusive()
    .one(&txn)
    .await?
  else {
    return Err(anyhow!("Segmentation state missing during persist").into());
  };

  let mut created = Vec::new();
  let mut max_closed_end_seq = None;
  let mut next_unsegmented_seq = state_model.next_unsegmented_seq;
  let mut last_boundary_context = state_model.last_closed_boundary_context.clone();

  for span in closed_spans {
    let span_id = Uuid::now_v7();
    let now = Utc::now();
    let source_model = build_episodic_projection(conversation_id, span_id, span).await?;
    let boundary_context = build_boundary_context(span, &source_model.title);

    episode_span::ActiveModel {
      id: Set(span_id),
      conversation_id: Set(conversation_id),
      start_seq: Set(span.start_seq),
      end_seq: Set(span.end_seq),
      boundary_reason: Set(span.boundary_reason.as_str().to_owned()),
      surprise_level: Set(span.surprise_level.as_str().to_owned()),
      status: Set("derived".to_owned()),
      created_at: Set(now.into()),
    }
    .insert(&txn)
    .await?;

    let episodic_active_model: episodic_memory::ActiveModel = source_model.to_model()?.into();
    episodic_memory::Entity::insert(episodic_active_model)
      .exec(&txn)
      .await?;

    max_closed_end_seq = Some(span.end_seq);
    next_unsegmented_seq = span.end_seq + 1;
    last_boundary_context = Some(serde_json::to_value(boundary_context)?);
    created.push(CreatedEpisode {
      id: source_model.id,
      surprise: source_model.surprise,
    });
  }

  let effective_eof_seen = state_model.eof_seen || eof_seen;
  let processed_all_seen_messages = state_model
    .last_seen_seq
    .is_none_or(|last_seen_seq| last_seen_seq <= claimed_until_seq);
  let can_finalize_tail_now = effective_eof_seen && processed_all_seen_messages;

  let open_tail_start_seq = if can_finalize_tail_now {
    None
  } else if let Some(last_closed_end_seq) = max_closed_end_seq {
    Some(last_closed_end_seq + 1)
  } else {
    Some(units.first().map_or(state_model.next_unsegmented_seq, |unit| unit.start_seq))
  };

  if can_finalize_tail_now && max_closed_end_seq.is_none() {
    next_unsegmented_seq = claimed_until_seq + 1;
  }

  let should_clear_eof = effective_eof_seen && open_tail_start_seq.is_none() && processed_all_seen_messages;
  let mut active_state: segmentation_state::ActiveModel = state_model.into_active_model();
  active_state.next_unsegmented_seq = Set(next_unsegmented_seq);
  active_state.open_tail_start_seq = Set(open_tail_start_seq);
  active_state.in_progress_until_seq = Set(None);
  active_state.in_progress_since = Set(None);
  active_state.eof_seen = Set(if should_clear_eof { false } else { effective_eof_seen });
  active_state.last_closed_boundary_context = Set(last_boundary_context);
  active_state.updated_at = Set(Utc::now().into());
  active_state.update(&txn).await?;

  txn.commit().await?;
  Ok(created)
}

async fn build_episodic_projection(
  conversation_id: Uuid,
  source_span_id: Uuid,
  span: &ClosedSpan,
) -> Result<plastmem_core::EpisodicMemory, AppError> {
  let now = Utc::now();
  let title = build_provisional_title(span);
  let content = build_provisional_content(span);
  let embedding_input = if title.is_empty() {
    content.clone()
  } else {
    format!("{title}. {content}")
  };
  let embedding = embed(&embedding_input).await?;
  let fsrs = FSRS::new(Some(&DEFAULT_PARAMETERS))?;
  let initial_states = fsrs.next_states(None, DESIRED_RETENTION, 0)?;
  let initial_state = initial_states.good.memory;
  let surprise = span.surprise_level.to_signal().clamp(0.0, 1.0);
  let boosted_stability = initial_state.stability * (1.0 + surprise * SURPRISE_BOOST_FACTOR);
  let start_at = span.messages.first().map_or(now, |message| message.timestamp);
  let end_at = span.messages.last().map_or(now, |message| message.timestamp);

  Ok(plastmem_core::EpisodicMemory {
    id: Uuid::now_v7(),
    conversation_id,
    source_span_id: Some(source_span_id),
    messages: span.messages.clone(),
    title,
    content,
    embedding,
    stability: boosted_stability,
    difficulty: initial_state.difficulty,
    surprise,
    start_at,
    end_at,
    created_at: now,
    last_reviewed_at: now,
    consolidated_at: None,
    derivation_status: "provisional".to_owned(),
  })
}

fn build_provisional_title(span: &ClosedSpan) -> String {
  let seed = span
    .messages
    .iter()
    .find(|message| !message.content.trim().is_empty())
    .map(|message| message.content.trim())
    .unwrap_or("Conversation segment");

  let words = seed
    .split_whitespace()
    .take(12)
    .collect::<Vec<_>>()
    .join(" ");
  if words.is_empty() {
    "Conversation segment".to_owned()
  } else {
    words
  }
}

fn build_provisional_content(span: &ClosedSpan) -> String {
  let mut lines = Vec::new();
  let mut current_header = None::<String>;

  for message in &span.messages {
    let header = message.timestamp.format("At: %b %d, %Y %l %p").to_string();
    if current_header.as_deref() != Some(header.as_str()) {
      lines.push(header.clone());
      current_header = Some(header);
    }
    lines.push(format!("* {} said {}", message.role, message.content.trim()));
  }

  lines.join("\n")
}

fn build_boundary_context(span: &ClosedSpan, title: &str) -> SegmentationBoundaryContext {
  let anchor_entities = top_keywords(&span.messages, 5);
  let last_turns_compact = span
    .messages
    .iter()
    .rev()
    .take(MAX_BOUNDARY_CONTEXT_LINES)
    .map(|message| format!("{}: {}", message.role, truncate(&message.content, 120)))
    .collect::<Vec<_>>()
    .into_iter()
    .rev()
    .collect();

  SegmentationBoundaryContext {
    anchor_topic: title.to_owned(),
    anchor_entities,
    last_turns_compact,
    boundary_style_hint: span.boundary_reason.as_str().to_owned(),
  }
}

async fn finalize_empty_pass(
  conversation_id: Uuid,
  db: &DatabaseConnection,
) -> Result<(), AppError> {
  let Some(model) = segmentation_state::Entity::find_by_id(conversation_id)
    .one(db)
    .await?
  else {
    return Ok(());
  };
  let mut active_model: segmentation_state::ActiveModel = model.into();
  active_model.in_progress_until_seq = Set(None);
  active_model.in_progress_since = Set(None);
  active_model.updated_at = Set(Utc::now().into());
  active_model.update(db).await?;
  Ok(())
}

async fn enqueue_pending_reviews(
  conversation_id: Uuid,
  context_messages: &[Message],
  db: &DatabaseConnection,
  review_storage: &PostgresStorage<MemoryReviewJob>,
) -> Result<(), AppError> {
  if !APP_ENV.enable_fsrs_review {
    return Ok(());
  }

  if let Some(pending_reviews) = MessageQueue::take_pending_reviews(conversation_id, db).await? {
    let review_job = MemoryReviewJob {
      pending_reviews,
      context_messages: context_messages.to_vec(),
      reviewed_at: Utc::now(),
    };
    let mut storage = review_storage.clone();
    storage.push(review_job).await?;
  }

  Ok(())
}

async fn enqueue_predict_calibrate_jobs(
  conversation_id: Uuid,
  episodes: &[CreatedEpisode],
  semantic_storage: &PostgresStorage<PredictCalibrateJob>,
) -> Result<(), AppError> {
  for episode in episodes {
    let mut storage = semantic_storage.clone();
    storage
      .push(PredictCalibrateJob {
        conversation_id,
        episode_id: episode.id,
        force: episode.surprise >= FLASHBULB_SURPRISE_THRESHOLD,
      })
      .await?;
  }
  Ok(())
}

async fn enqueue_follow_up_if_needed(
  conversation_id: Uuid,
  db: &DatabaseConnection,
  segmentation_storage: &PostgresStorage<EventSegmentationJob>,
) -> Result<(), AppError> {
  let txn = db.begin().await?;
  let Some(model) = segmentation_state::Entity::find_by_id(conversation_id)
    .lock_exclusive()
    .one(&txn)
    .await?
  else {
    return Ok(());
  };

  let messages_pending = match model.last_seen_seq {
    Some(last_seen_seq) if last_seen_seq >= model.next_unsegmented_seq => {
      last_seen_seq - model.next_unsegmented_seq + 1
    }
    _ => 0,
  };

  if model.in_progress_until_seq.is_some() {
    txn.commit().await?;
    return Ok(());
  }

  let should_schedule = model.last_seen_seq.is_some()
    && (model.eof_seen || messages_pending >= SEGMENTATION_WINDOW_BASE);
  if !should_schedule {
    txn.commit().await?;
    return Ok(());
  }

  let last_seen_seq = model.last_seen_seq;
  let mut active_model: segmentation_state::ActiveModel = model.into();
  active_model.in_progress_until_seq = Set(last_seen_seq);
  active_model.in_progress_since = Set(Some(Utc::now().into()));
  active_model.updated_at = Set(Utc::now().into());
  active_model.update(&txn).await?;
  txn.commit().await?;

  let mut storage = segmentation_storage.clone();
  storage.push(EventSegmentationJob { conversation_id }).await?;
  Ok(())
}

fn format_message_line(record: &ConversationMessageRecord) -> String {
  format!(
    "[seq={}] {} [{}] {}",
    record.seq,
    record.message.timestamp.format("%Y-%m-%dT%H:%M:%SZ"),
    record.message.role,
    record.message.content
  )
}

fn tokenize(text: &str) -> BTreeSet<String> {
  text
    .split(|char: char| !char.is_alphanumeric())
    .filter(|token| token.len() >= 3)
    .map(|token| token.to_ascii_lowercase())
    .collect()
}

fn top_keywords(messages: &[Message], limit: usize) -> Vec<String> {
  let mut seen = BTreeSet::new();
  for token in messages.iter().flat_map(|message| tokenize(&message.content).into_iter()) {
    if seen.len() >= limit {
      break;
    }
    seen.insert(token);
  }
  seen.into_iter().collect()
}

fn unit_surface_text(unit: &AnalysisUnit) -> String {
  unit
    .messages
    .iter()
    .map(|record| record.message.content.trim())
    .filter(|content| !content.is_empty())
    .collect::<Vec<_>>()
    .join(" ")
}

fn aggregate_window_text(units: &[AnalysisUnit]) -> String {
  units
    .iter()
    .map(unit_surface_text)
    .filter(|text| !text.is_empty())
    .collect::<Vec<_>>()
    .join(" ")
}

fn aggregate_token_set(units: &[AnalysisUnit]) -> BTreeSet<String> {
  let mut tokens = BTreeSet::new();
  for unit in units {
    tokens.extend(unit.token_set.iter().cloned());
  }
  tokens
}

fn span_surface_text(span: &ClosedSpan) -> String {
  span
    .messages
    .iter()
    .map(|message| message.content.trim())
    .filter(|content| !content.is_empty())
    .collect::<Vec<_>>()
    .join(" ")
}

fn span_token_set(span: &ClosedSpan) -> BTreeSet<String> {
  span
    .messages
    .iter()
    .flat_map(|message| tokenize(&message.content))
    .collect()
}

fn truncate(text: &str, max_len: usize) -> String {
  if text.len() <= max_len {
    return text.to_owned();
  }

  let mut truncated = text.chars().take(max_len).collect::<String>();
  truncated.push_str("...");
  truncated
}

fn structural_boundary_cue(_previous: &AnalysisUnit, _current: &AnalysisUnit) -> f32 {
  // Keep the cue_phrase slot in the scoring model, but avoid language-specific
  // lexical cue lists. We currently rely on language-agnostic structural
  // signals from other channels instead of hard-coded phrases.
  0.0
}

fn ends_with_question_marker(text: &str) -> bool {
  let trimmed = text.trim_end();
  trimmed.ends_with('?') || trimmed.ends_with('？')
}

fn char_ngram_overlap(left: &str, right: &str) -> f32 {
  let left_ngrams = char_ngrams(left, 3);
  let right_ngrams = char_ngrams(right, 3);
  if left_ngrams.is_empty() || right_ngrams.is_empty() {
    return 0.0;
  }

  let overlap = left_ngrams
    .iter()
    .filter(|ngram| right_ngrams.contains(*ngram))
    .count() as f32;
  let union = left_ngrams.union(&right_ngrams).count() as f32;
  if union <= 0.0 {
    0.0
  } else {
    overlap / union
  }
}

fn char_ngrams(text: &str, n: usize) -> BTreeSet<String> {
  let normalized = text
    .chars()
    .filter(|ch| ch.is_alphanumeric())
    .flat_map(|ch| ch.to_lowercase())
    .collect::<Vec<_>>();

  if normalized.len() < n {
    return BTreeSet::new();
  }

  normalized
    .windows(n)
    .map(|window| window.iter().collect::<String>())
    .collect()
}

fn to_messages(records: &[ConversationMessageRecord]) -> Vec<Message> {
  records.iter().map(|record| record.message.clone()).collect()
}

#[cfg(test)]
mod tests {
  use super::*;
  use chrono::TimeZone;
  use plastmem_shared::MessageRole;

  fn record(seq: i64, role: &str, content: &str) -> ConversationMessageRecord {
    ConversationMessageRecord {
      id: Uuid::now_v7(),
      conversation_id: Uuid::nil(),
      seq,
      message: Message {
        role: MessageRole(role.to_owned()),
        content: content.to_owned(),
        timestamp: Utc.timestamp_opt(seq, 0).single().expect("valid timestamp"),
      },
      created_at: Utc.timestamp_opt(seq, 0).single().expect("valid timestamp"),
    }
  }

  fn record_at(seq: i64, timestamp: i64, role: &str, content: &str) -> ConversationMessageRecord {
    ConversationMessageRecord {
      id: Uuid::now_v7(),
      conversation_id: Uuid::nil(),
      seq,
      message: Message {
        role: MessageRole(role.to_owned()),
        content: content.to_owned(),
        timestamp: Utc
          .timestamp_opt(timestamp, 0)
          .single()
          .expect("valid timestamp"),
      },
      created_at: Utc
        .timestamp_opt(timestamp, 0)
        .single()
        .expect("valid timestamp"),
    }
  }

  fn message(role: &str, content: &str, timestamp: i64) -> Message {
    Message {
      role: MessageRole(role.to_owned()),
      content: content.to_owned(),
      timestamp: Utc
        .timestamp_opt(timestamp, 0)
        .single()
        .expect("valid timestamp"),
    }
  }

  #[test]
  fn candidate_scorer_skips_plain_question_answer_exchange() {
    let units = build_analysis_units(&[
      record(0, "John", "What game are you playing right now?"),
      record(1, "James", "I'm playing The Witcher 3 at the moment."),
      record(2, "John", "Nice, I keep hearing good things about it."),
    ]);

    let candidates = score_candidate_boundaries(&units, None);
    assert!(candidates.is_empty());
  }

  #[test]
  fn candidate_scorer_keeps_temporal_gap_boundary() {
    let units = build_analysis_units(&[
      record_at(0, 0, "John", "What game are you playing right now?"),
      record_at(1, 60, "James", "I'm playing The Witcher 3 at the moment."),
      record_at(2, 60 * 60 * 4, "John", "How was your trip last weekend?"),
    ]);

    let candidates = score_candidate_boundaries(&units, None);
    assert!(candidates.iter().any(|candidate| candidate.next_unit_index == 2));
  }

  #[test]
  fn candidate_scorer_keeps_clear_same_session_topic_shift() {
    let units = build_analysis_units(&[
      record(0, "John", "I've been playing The Witcher 3 a lot this week."),
      record(1, "James", "Nice, I love open world games and long story quests."),
      record(2, "John", "My two dogs kept me busy at the park this morning."),
      record(3, "James", "Dogs always make the day better, especially energetic ones."),
    ]);

    let candidates = score_candidate_boundaries(&units, None);
    assert!(candidates.iter().any(|candidate| candidate.next_unit_index == 2));
  }

  #[test]
  fn candidate_scorer_suppresses_dense_same_session_cluster() {
    let units = build_analysis_units(&[
      record(12, "John", "Aww, they're adorable! What are the names of your pets? And what are your plans for the app?"),
      record(13, "James", "Max and Daisy. The goal is to connect pet owners with reliable dog walkers and provide helpful pet care guidance."),
      record(14, "John", "Sounds good, James! What sets it apart from other existing apps?"),
      record(15, "James", "The personal touch really sets it apart. Users can add their pup's preferences and needs."),
      record(16, "John", "That's a great idea! What motivates you to work on your programming projects?"),
      record(17, "James", "Creating something and seeing it come to life gives me a great sense of accomplishment."),
      record(18, "John", "What are you working on that has you feeling so accomplished?"),
      record(19, "James", "I'm working on a game project I've wanted to make since I was a kid."),
    ]);

    let candidates = score_candidate_boundaries(&units, None);
    let dense_candidates = candidates
      .iter()
      .filter(|candidate| candidate.time_gap == 0.0 && candidate.cue_phrase == 0.0)
      .count();
    assert!(dense_candidates <= 1, "dense candidates: {candidates:?}");
  }

  #[test]
  fn normalize_closed_spans_merges_weak_singleton_into_more_affine_neighbor() {
    let spans = vec![
      ClosedSpan {
        start_seq: 0,
        end_seq: 1,
        messages: vec![
          message("John", "My dogs had a checkup at the vet today.", 0),
          message("James", "I hope Max and Daisy are doing well.", 1),
        ],
        boundary_reason: BoundaryReason::SessionBreak,
        surprise_level: SurpriseLevel::Low,
      },
      ClosedSpan {
        start_seq: 2,
        end_seq: 2,
        messages: vec![message("John", "The vet said Daisy needs more exercise.", 2)],
        boundary_reason: BoundaryReason::TopicShift,
        surprise_level: SurpriseLevel::Low,
      },
      ClosedSpan {
        start_seq: 3,
        end_seq: 4,
        messages: vec![
          message("James", "I went bowling after work yesterday.", 3),
          message("John", "Bowling sounds fun for a weekend outing.", 4),
        ],
        boundary_reason: BoundaryReason::TopicShift,
        surprise_level: SurpriseLevel::Low,
      },
    ];

    let normalized = normalize_closed_spans(spans);
    assert_eq!(normalized.len(), 2);
    assert_eq!(normalized[0].start_seq, 0);
    assert_eq!(normalized[0].end_seq, 2);
    assert_eq!(normalized[1].start_seq, 3);
  }

  #[test]
  fn normalize_closed_spans_can_merge_short_high_span_when_affinity_is_clear() {
    let spans = vec![
      ClosedSpan {
        start_seq: 0,
        end_seq: 1,
        messages: vec![
          message("John", "My dogs had a checkup at the vet today.", 0),
          message("James", "The vet said Daisy is healthy and just needs more exercise.", 1),
        ],
        boundary_reason: BoundaryReason::SessionBreak,
        surprise_level: SurpriseLevel::Low,
      },
      ClosedSpan {
        start_seq: 2,
        end_seq: 3,
        messages: vec![
          message("John", "Daisy needs more exercise after the vet visit.", 2),
          message("James", "I should probably walk Daisy more after dinner.", 3),
        ],
        boundary_reason: BoundaryReason::TopicShift,
        surprise_level: SurpriseLevel::High,
      },
      ClosedSpan {
        start_seq: 4,
        end_seq: 5,
        messages: vec![
          message("John", "I went bowling after work yesterday.", 4),
          message("James", "Bowling sounds fun for a weekend outing.", 5),
        ],
        boundary_reason: BoundaryReason::TopicShift,
        surprise_level: SurpriseLevel::Low,
      },
    ];

    let normalized = normalize_closed_spans(spans);
    assert_eq!(normalized.len(), 2);
    assert_eq!(normalized[0].start_seq, 0);
    assert_eq!(normalized[0].end_seq, 3);
    assert_eq!(normalized[1].start_seq, 4);
  }
}
