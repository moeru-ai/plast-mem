use std::collections::{BTreeMap, BTreeSet, HashSet};

use chrono::{DateTime, TimeDelta, Utc};
use plastmem_ai::{
  ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
  ChatCompletionRequestUserMessage, cosine_similarity, embed_many, generate_object,
};
use plastmem_core::{ConversationMessageRecord, SegmentationBoundaryContext};
use plastmem_shared::{AppError, Message};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[cfg(any(test, feature = "segmentation-debug"))]
use uuid::Uuid;

const MAX_BOUNDARY_CONTEXT_LINES: usize = 3;
const MIN_SURPRISE_SHIFT_SEGMENT_MESSAGES: usize = 5;
const MIN_SURPRISE_SHIFT_SEGMENT_UNITS: usize = 2;
const MIN_OBSERVED_TAIL_MESSAGES_FOR_NON_TEMPORAL_CLOSE: usize = 3;
const MIN_TOPIC_SHIFT_BOUNDARY_STRENGTH: f32 = 1.95;
const MIN_INTENT_SHIFT_BOUNDARY_STRENGTH: f32 = 2.05;
const MIN_SURPRISE_SHIFT_BOUNDARY_STRENGTH: f32 = 1.9;
const PRE_GAP_TRAILING_REPLY_MAX_CHARS: usize = 120;
const PRE_GAP_TRAILING_REPLY_AFFINITY_THRESHOLD: f32 = 0.08;
const PRE_GAP_TRAILING_REPLY_AFFINITY_MARGIN: f32 = 0.04;
const PRE_GAP_TRAILING_REPLY_PREVIOUS_MAX_MINUTES: i64 = 10;
const PRE_GAP_TRAILING_REPLY_NEXT_MIN_MINUTES: i64 = 60;
const BOUNDARY_PLANNER_MIN_WINDOW_MESSAGES: usize = 30;
const BOUNDARY_PLANNER_TEMPORAL_GAP_MIN_MINUTES: i64 = 60;
const BOUNDARY_PLANNER_MIN_SEGMENT_MESSAGES: usize = 4;
const BOUNDARY_PLANNER_MIN_HINT_SPACING: usize = 5;
const BOUNDARY_PLANNER_EMBEDDING_DROP_THRESHOLD: f32 = 0.18;
const BOUNDARY_PLANNER_SYSTEM_PROMPT: &str = r#"
You are planning coarse episodic boundaries inside one same-session conversation window.
Return only JSON that matches the schema.

Choose only boundaries that split the window into coherent, independently retrievable event chunks.
Do not summarize or rewrite the conversation.

Rules:
- A boundary must land on the first unit of the new segment.
- Prefer a small number of meaningful boundaries over many local turn-level cuts.
- For windows longer than about 16 messages, actively look for 1-3 semantic event boundaries.
- For windows longer than about 24 messages, return at least one boundary unless every candidate hint would only split a trivial question-answer exchange.
- Return no boundary only when the whole window is clearly one continuous activity or one tightly focused topic and the candidate hints are poor.
- If the user message includes candidate hints, prefer selecting 1-3 high-quality boundaries from those hints instead of inventing unrelated cut points.
- When candidate hints are available, every boundary `next_unit_index` must be one of the hinted indexes.
- Return distinct boundary indexes in increasing order. Do not repeat the same boundary.
- Never use `next_unit_index` 0.
- Use `topic_shift` for normal same-session semantic cuts. Do not use `temporal_gap` or `session_break` inside this same-session window.
- Use `surprise_shift` and `extremely_high` only for genuinely abrupt, high-impact life events, not ordinary topic changes.
- Do NOT split ordinary question-answer exchanges, acknowledgements, short follow-ups, or one-off asides.
- Same-session topic shifts are valid only when both sides are substantial event chunks.
- Useful boundaries often occur when the conversation moves between different activities, projects, relationships, life events, plans, places, media, work, health, pets, or trips.
- Do not create boundaries for greetings, closings, or normal conversational handoff unless the subject materially changes.
- Avoid creating 1-2 message segments unless there is a clear session break or extreme surprise.
- `boundary_reason` must be exactly one of:
  `topic_shift`, `intent_shift`, `surprise_shift`, `temporal_gap`, `session_break`
- `surprise_level` must be exactly one of:
  `low`, `high`, `extremely_high`
"#;
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SurpriseLevel {
  Low,
  High,
  ExtremelyHigh,
}

impl SurpriseLevel {
  pub const fn to_signal(self) -> f32 {
    match self {
      Self::Low => 0.2,
      Self::High => 0.6,
      Self::ExtremelyHigh => 0.9,
    }
  }

  pub const fn as_str(self) -> &'static str {
    match self {
      Self::Low => "low",
      Self::High => "high",
      Self::ExtremelyHigh => "extremely_high",
    }
  }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryReason {
  TopicShift,
  IntentShift,
  SurpriseShift,
  TemporalGap,
  SessionBreak,
}

impl BoundaryReason {
  pub const fn as_str(self) -> &'static str {
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
struct BoundaryPlannerOutput {
  boundaries: Vec<PlannedBoundary>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct PlannedBoundary {
  next_unit_index: usize,
  boundary_reason: BoundaryReason,
  surprise_level: SurpriseLevel,
  confidence: f32,
}

#[derive(Debug, Clone)]
pub struct AnalysisUnit {
  pub start_seq: i64,
  pub end_seq: i64,
  pub messages: Vec<ConversationMessageRecord>,
  joined_content: String,
  token_set: BTreeSet<String>,
  pub start_at: DateTime<Utc>,
  pub end_at: DateTime<Utc>,
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
  candidate_score: f32,
  time_gap: f32,
  semantic_drop: f32,
  online_surprise_prior: f32,
  micro_exchange_penalty: f32,
}

#[derive(Debug, Clone)]
pub struct ClosedSpan {
  pub start_seq: i64,
  pub end_seq: i64,
  pub messages: Vec<Message>,
  pub boundary_reason: BoundaryReason,
  pub surprise_level: SurpriseLevel,
}

#[cfg(any(test, feature = "segmentation-debug"))]
#[derive(Debug, Clone, Serialize)]
pub struct DebugSegmentationSpan {
  pub start_seq: i64,
  pub end_seq: i64,
  pub message_count: usize,
  pub total_chars: usize,
  pub boundary_reason: String,
  pub surprise_level: String,
  pub start_at: DateTime<Utc>,
  pub end_at: DateTime<Utc>,
  pub first_message: String,
  pub last_message: String,
  pub messages: Vec<DebugSegmentationMessage>,
}

#[cfg(any(test, feature = "segmentation-debug"))]
#[derive(Debug, Clone, Serialize)]
pub struct DebugSegmentationMessage {
  pub seq: i64,
  pub role: String,
  pub content: String,
  pub timestamp: DateTime<Utc>,
}

#[cfg(any(test, feature = "segmentation-debug"))]
#[derive(Debug, Clone, Copy)]
pub enum DebugSegmentationMode {
  Deterministic,
  PlannerOnly,
  EmbeddingPlanner,
  FullLlm,
}

#[cfg(any(test, feature = "segmentation-debug"))]
#[derive(Debug, Clone, Serialize)]
pub struct DebugSegmentationTrace {
  pub deterministic_candidates: Vec<DebugBoundaryTrace>,
  pub planned_boundaries: Vec<DebugBoundaryTrace>,
  pub merged_boundaries: Vec<DebugBoundaryTrace>,
  pub spans: Vec<DebugSegmentationSpan>,
}

#[cfg(any(test, feature = "segmentation-debug"))]
#[derive(Debug, Clone, Serialize)]
pub struct DebugBoundaryTrace {
  pub source: String,
  pub next_unit_index: usize,
  pub next_seq: i64,
  pub previous_seq: i64,
  pub boundary_reason: Option<String>,
  pub surprise_level: Option<String>,
  pub candidate_score: f32,
  pub time_gap: f32,
  pub semantic_drop: f32,
  pub online_surprise_prior: f32,
  pub micro_exchange_penalty: f32,
}

#[derive(Debug, Clone)]
pub struct SegmentationOutput {
  pub analysis_units: Vec<AnalysisUnit>,
  pub closed_spans: Vec<ClosedSpan>,
}

pub async fn segment_records(
  records: &[ConversationMessageRecord],
  boundary_context: Option<&SegmentationBoundaryContext>,
  has_open_tail: bool,
  eof_seen: bool,
) -> Result<SegmentationOutput, AppError> {
  let analysis_units = build_analysis_units(records);
  let candidates = score_candidate_boundaries(&analysis_units, boundary_context);
  let refined_candidates = refine_temporal_fallback_boundaries(&candidates);
  let planned_boundaries = plan_embedding_window_boundaries(&analysis_units, boundary_context)
    .await
    .or_else(|error| {
      tracing::warn!(
        "Embedding planner failed, falling back to deterministic window planner: {error}"
      );
      Ok::<_, AppError>(Vec::new())
    })?;
  let refined_boundaries = merge_refined_boundaries(refined_candidates, planned_boundaries);
  let closed_spans = close_segments(
    &analysis_units,
    &refined_boundaries,
    has_open_tail,
    eof_seen,
  );

  Ok(SegmentationOutput {
    analysis_units,
    closed_spans,
  })
}

#[cfg(any(test, feature = "segmentation-debug"))]
pub async fn debug_segment_messages(
  messages: Vec<Message>,
  mode: DebugSegmentationMode,
) -> Result<Vec<DebugSegmentationSpan>, AppError> {
  Ok(
    debug_segment_messages_with_trace(messages, mode)
      .await?
      .spans,
  )
}

#[cfg(any(test, feature = "segmentation-debug"))]
pub async fn debug_segment_messages_with_trace(
  messages: Vec<Message>,
  mode: DebugSegmentationMode,
) -> Result<DebugSegmentationTrace, AppError> {
  let conversation_id = Uuid::now_v7();
  let records = messages
    .into_iter()
    .enumerate()
    .map(|(index, message)| ConversationMessageRecord {
      id: Uuid::now_v7(),
      conversation_id,
      seq: index as i64,
      message,
      created_at: Utc::now(),
    })
    .collect::<Vec<_>>();

  let analysis_units = build_analysis_units(&records);
  let candidates = score_candidate_boundaries(&analysis_units, None);
  let deterministic_candidates = candidates
    .iter()
    .map(|candidate| debug_candidate_boundary(&analysis_units, candidate))
    .collect();
  let fallback_candidates = refine_candidates_with_fallback(&candidates);
  let (planned_boundaries, refined_boundaries) = match mode {
    DebugSegmentationMode::Deterministic => (Vec::new(), fallback_candidates),
    DebugSegmentationMode::PlannerOnly => {
      let planned_boundaries = plan_embedding_window_boundaries(&analysis_units, None).await?;
      let refined_boundaries =
        merge_refined_boundaries(fallback_candidates, planned_boundaries.clone());
      (planned_boundaries, refined_boundaries)
    }
    DebugSegmentationMode::EmbeddingPlanner => {
      let planned_boundaries = plan_embedding_window_boundaries(&analysis_units, None).await?;
      let refined_boundaries =
        merge_refined_boundaries(fallback_candidates, planned_boundaries.clone());
      (planned_boundaries, refined_boundaries)
    }
    DebugSegmentationMode::FullLlm => {
      let refined_candidates = refine_temporal_fallback_boundaries(&candidates);
      let planned_boundaries = plan_embedding_window_boundaries(&analysis_units, None).await?;
      let refined_boundaries =
        merge_refined_boundaries(refined_candidates, planned_boundaries.clone());
      (planned_boundaries, refined_boundaries)
    }
  };

  let closed_spans = close_segments(&analysis_units, &refined_boundaries, false, true);

  let planned_boundaries = planned_boundaries
    .iter()
    .map(|boundary| debug_refined_boundary("planner", &analysis_units, boundary))
    .collect();
  let merged_boundaries = refined_boundaries
    .iter()
    .map(|boundary| debug_refined_boundary("merged", &analysis_units, boundary))
    .collect();

  let spans = closed_spans
    .into_iter()
    .map(|span| {
      let start_at = span
        .messages
        .first()
        .map_or_else(Utc::now, |message| message.timestamp);
      let end_at = span
        .messages
        .last()
        .map_or(start_at, |message| message.timestamp);
      let first_message = span
        .messages
        .first()
        .map_or_else(String::new, debug_message_preview);
      let last_message = span
        .messages
        .last()
        .map_or_else(String::new, debug_message_preview);
      let total_chars = span
        .messages
        .iter()
        .map(|message| message.content.len())
        .sum();

      DebugSegmentationSpan {
        start_seq: span.start_seq,
        end_seq: span.end_seq,
        message_count: span.messages.len(),
        total_chars,
        boundary_reason: span.boundary_reason.as_str().to_owned(),
        surprise_level: span.surprise_level.as_str().to_owned(),
        start_at,
        end_at,
        first_message,
        last_message,
        messages: span
          .messages
          .iter()
          .enumerate()
          .map(|(index, message)| DebugSegmentationMessage {
            seq: span.start_seq + index as i64,
            role: message.role.to_string(),
            content: message.content.clone(),
            timestamp: message.timestamp,
          })
          .collect(),
      }
    })
    .collect();

  Ok(DebugSegmentationTrace {
    deterministic_candidates,
    planned_boundaries,
    merged_boundaries,
    spans,
  })
}

#[cfg(any(test, feature = "segmentation-debug"))]
fn refine_candidates_with_fallback(candidates: &[CandidateBoundary]) -> Vec<RefinedBoundary> {
  refine_temporal_fallback_boundaries(candidates)
}

fn refine_temporal_fallback_boundaries(candidates: &[CandidateBoundary]) -> Vec<RefinedBoundary> {
  candidates
    .iter()
    .filter(|candidate| candidate.time_gap >= 0.8)
    .map(|candidate| RefinedBoundary {
      next_unit_index: candidate.next_unit_index,
      boundary_reason: BoundaryReason::TemporalGap,
      surprise_level: if candidate.time_gap >= 1.0 {
        SurpriseLevel::ExtremelyHigh
      } else {
        SurpriseLevel::High
      },
      candidate_score: candidate.score,
      time_gap: candidate.time_gap,
      semantic_drop: candidate.semantic_drop,
      online_surprise_prior: candidate.online_surprise_prior,
      micro_exchange_penalty: candidate.micro_exchange_penalty,
    })
    .collect()
}

#[cfg(any(test, feature = "segmentation-debug"))]
fn debug_candidate_boundary(
  units: &[AnalysisUnit],
  candidate: &CandidateBoundary,
) -> DebugBoundaryTrace {
  DebugBoundaryTrace {
    source: "deterministic_candidate".to_owned(),
    next_unit_index: candidate.next_unit_index,
    next_seq: units[candidate.next_unit_index].start_seq,
    previous_seq: units[candidate.next_unit_index - 1].end_seq,
    boundary_reason: None,
    surprise_level: None,
    candidate_score: candidate.score,
    time_gap: candidate.time_gap,
    semantic_drop: candidate.semantic_drop,
    online_surprise_prior: candidate.online_surprise_prior,
    micro_exchange_penalty: candidate.micro_exchange_penalty,
  }
}

#[cfg(any(test, feature = "segmentation-debug"))]
fn debug_refined_boundary(
  source: &str,
  units: &[AnalysisUnit],
  boundary: &RefinedBoundary,
) -> DebugBoundaryTrace {
  DebugBoundaryTrace {
    source: source.to_owned(),
    next_unit_index: boundary.next_unit_index,
    next_seq: units[boundary.next_unit_index].start_seq,
    previous_seq: units[boundary.next_unit_index - 1].end_seq,
    boundary_reason: Some(boundary.boundary_reason.as_str().to_owned()),
    surprise_level: Some(boundary.surprise_level.as_str().to_owned()),
    candidate_score: boundary.candidate_score,
    time_gap: boundary.time_gap,
    semantic_drop: boundary.semantic_drop,
    online_surprise_prior: boundary.online_surprise_prior,
    micro_exchange_penalty: boundary.micro_exchange_penalty,
  }
}

fn build_analysis_units(records: &[ConversationMessageRecord]) -> Vec<AnalysisUnit> {
  let mut units = Vec::new();

  for record in records {
    let should_merge = units
      .last()
      .is_some_and(|unit| should_merge_into_unit(unit, record));
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
    let candidate = candidate_boundary_for_index(units, boundary_context, next_unit_index);

    let strong_context_free_candidate = candidate.semantic_drop >= 0.76
      && candidate.online_surprise_prior >= 0.18
      && candidate.micro_exchange_penalty < 0.25;
    if candidate.score >= 1.45
      || candidate.time_gap >= 0.8
      || candidate.cue_phrase >= 0.7
      || strong_context_free_candidate
    {
      candidates.push(candidate);
    }
  }

  suppress_dense_same_session_candidates(candidates)
}

fn candidate_boundary_for_index(
  units: &[AnalysisUnit],
  boundary_context: Option<&SegmentationBoundaryContext>,
  next_unit_index: usize,
) -> CandidateBoundary {
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

  let micro_exchange_penalty = micro_exchange_penalty(units, next_unit_index, time_gap, cue_phrase);
  let adjacent_drop = semantic_drop_score(previous, current);
  let window_drop = window_semantic_drop(units, next_unit_index);
  let semantic_drop =
    (adjacent_drop.max(window_drop) - micro_exchange_penalty * 0.35).clamp(0.0, 1.0);
  let online_surprise_prior = online_surprise_prior(
    previous,
    current,
    boundary_context,
    time_gap,
    cue_phrase,
    semantic_drop,
  );
  let score = (time_gap + cue_phrase + semantic_drop + online_surprise_prior
    - micro_exchange_penalty)
    .max(0.0);

  CandidateBoundary {
    next_unit_index,
    score,
    time_gap,
    cue_phrase,
    semantic_drop,
    online_surprise_prior,
    micro_exchange_penalty,
  }
}

fn suppress_dense_same_session_candidates(
  candidates: Vec<CandidateBoundary>,
) -> Vec<CandidateBoundary> {
  let mut filtered: Vec<CandidateBoundary> = Vec::new();
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
        && next
          .next_unit_index
          .saturating_sub(candidates[end - 1].next_unit_index)
          <= 2;
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
  let char_drop =
    1.0 - char_ngram_overlap(&unit_surface_text(previous), &unit_surface_text(current));
  (0.6 * token_drop + 0.4 * char_drop).clamp(0.0, 1.0)
}

fn lexical_set_distance(previous: &BTreeSet<String>, current: &BTreeSet<String>) -> f32 {
  if previous.is_empty() || current.is_empty() {
    return 0.1;
  }

  let overlap = previous
    .iter()
    .filter(|token| current.contains(*token))
    .count() as f32;
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

  (0.4 * time_gap
    + 0.25 * cue_phrase
    + 0.15 * semantic_drop
    + 0.15 * context_novelty
    + speaker_shift)
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

async fn plan_embedding_window_boundaries(
  units: &[AnalysisUnit],
  boundary_context: Option<&SegmentationBoundaryContext>,
) -> Result<Vec<RefinedBoundary>, AppError> {
  if units.len() < 2 {
    return Ok(Vec::new());
  }

  let mut planned = Vec::new();
  for (window_start, window_end) in planner_windows(units) {
    let window = &units[window_start..window_end];
    let window_message_count = count_messages(window);
    if window_message_count < BOUNDARY_PLANNER_MIN_WINDOW_MESSAGES {
      continue;
    }

    let hint_candidates = match embedding_planner_hint_candidates(window, boundary_context).await {
      Ok(candidates) => candidates,
      Err(error) => {
        tracing::warn!(
          window_start,
          window_end,
          "Embedding boundary hint generation failed, continuing without planner: {error}"
        );
        #[cfg(feature = "segmentation-debug")]
        eprintln!(
          "Embedding boundary hint generation failed for units {window_start}..{window_end}: {error}"
        );
        continue;
      }
    };
    if hint_candidates.is_empty() {
      continue;
    }

    let candidate_hints = format_planner_candidate_hints(window, &hint_candidates);
    let valid_hint_indexes = hint_candidates
      .iter()
      .map(|candidate| candidate.next_unit_index)
      .collect::<BTreeSet<_>>();
    let output = match request_window_boundary_plan_with_hints(
      window,
      boundary_context,
      candidate_hints,
    )
    .await
    {
      Ok(output) => output,
      Err(error) => {
        tracing::warn!(
          window_start,
          window_end,
          "Embedding window boundary planning failed, continuing without planner: {error}"
        );
        #[cfg(feature = "segmentation-debug")]
        eprintln!(
          "Embedding window boundary planning failed for units {window_start}..{window_end}: {error}"
        );
        continue;
      }
    };

    for boundary in output.boundaries.into_iter().take(3) {
      if boundary.next_unit_index == 0
        || boundary.next_unit_index >= window.len()
        || !valid_hint_indexes.contains(&boundary.next_unit_index)
      {
        #[cfg(feature = "segmentation-debug")]
        eprintln!(
          "Ignoring invalid embedding planned boundary for units {window_start}..{window_end}: next_unit_index={} reason={} surprise={} confidence={:.2}",
          boundary.next_unit_index,
          boundary.boundary_reason.as_str(),
          boundary.surprise_level.as_str(),
          boundary.confidence,
        );
        continue;
      }

      let next_unit_index = window_start + boundary.next_unit_index;
      let candidate = hint_candidates
        .iter()
        .find(|candidate| candidate.next_unit_index == boundary.next_unit_index)
        .cloned()
        .unwrap_or_else(|| candidate_boundary_for_index(units, boundary_context, next_unit_index));
      let confidence = boundary.confidence.clamp(0.0, 1.0);
      let planner_score = 1.35 + confidence * 0.75;
      let (boundary_reason, surprise_level) = normalize_planned_boundary_kind(boundary);
      planned.push(RefinedBoundary {
        next_unit_index,
        boundary_reason,
        surprise_level,
        candidate_score: candidate.score.max(planner_score),
        time_gap: candidate.time_gap,
        semantic_drop: candidate.semantic_drop,
        online_surprise_prior: candidate.online_surprise_prior.max(confidence * 0.35),
        micro_exchange_penalty: candidate.micro_exchange_penalty,
      });
    }
  }

  Ok(planned)
}

fn planner_windows(units: &[AnalysisUnit]) -> Vec<(usize, usize)> {
  let mut windows = Vec::new();
  if units.is_empty() {
    return windows;
  }

  let mut start = 0usize;
  for next_unit_index in 1..units.len() {
    let gap_minutes =
      (units[next_unit_index].start_at - units[next_unit_index - 1].end_at).num_minutes();
    if gap_minutes >= BOUNDARY_PLANNER_TEMPORAL_GAP_MIN_MINUTES {
      windows.push((start, next_unit_index));
      start = next_unit_index;
    }
  }
  windows.push((start, units.len()));

  windows
}

async fn request_window_boundary_plan_with_hints(
  units: &[AnalysisUnit],
  boundary_context: Option<&SegmentationBoundaryContext>,
  candidate_hints: String,
) -> Result<BoundaryPlannerOutput, AppError> {
  let unit_lines = units
    .iter()
    .enumerate()
    .map(|(index, unit)| {
      format!(
        "[unit {index}] seq {}..{} time {}..{}\n{}",
        unit.start_seq,
        unit.end_seq,
        unit.start_at.to_rfc3339(),
        unit.end_at.to_rfc3339(),
        unit.joined_content,
      )
    })
    .collect::<Vec<_>>()
    .join("\n\n");

  let boundary_context = boundary_context.map_or_else(
    || "none".to_owned(),
    |context| serde_json::to_string(context).unwrap_or_else(|_| "none".to_owned()),
  );

  let user = format!(
    "Previous boundary context: {boundary_context}\n\n\
     Candidate boundary hints:\n{candidate_hints}\n\n\
     Conversation window units:\n{unit_lines}\n\n\
     Return up to 3 distinct `boundaries` using `next_unit_index`, where the index is the first unit of the new segment. \
     Prefer 2-3 boundaries when the window clearly contains multiple activities, but return an empty array for a single coherent event."
  );

  generate_object::<BoundaryPlannerOutput>(
    vec![
      ChatCompletionRequestMessage::System(ChatCompletionRequestSystemMessage::from(
        BOUNDARY_PLANNER_SYSTEM_PROMPT.trim(),
      )),
      ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage::from(user)),
    ],
    "boundary_planner_output".to_owned(),
    Some(
      "A coarse same-session event boundary plan with zero to three distinct boundaries."
        .to_owned(),
    ),
  )
  .await
}

fn format_planner_candidate_hints(
  units: &[AnalysisUnit],
  candidates: &[CandidateBoundary],
) -> String {
  let hints = candidates
    .iter()
    .map(|candidate| {
      format!(
        "- next_unit_index={} seq {} after {} score={:.2} semantic_drop={:.2} penalty={:.2}",
        candidate.next_unit_index,
        units[candidate.next_unit_index].start_seq,
        units[candidate.next_unit_index - 1].end_seq,
        candidate.score,
        candidate.semantic_drop,
        candidate.micro_exchange_penalty,
      )
    })
    .collect::<Vec<_>>();

  if hints.is_empty() {
    "none".to_owned()
  } else {
    let indexes = hints
      .iter()
      .filter_map(|hint| {
        hint
          .strip_prefix("- next_unit_index=")
          .and_then(|rest| rest.split_whitespace().next())
      })
      .collect::<Vec<_>>()
      .join(", ");
    format!(
      "Valid next_unit_index choices: {indexes}\n{}",
      hints.join("\n")
    )
  }
}

async fn embedding_planner_hint_candidates(
  units: &[AnalysisUnit],
  boundary_context: Option<&SegmentationBoundaryContext>,
) -> Result<Vec<CandidateBoundary>, AppError> {
  let total_messages = count_messages(units);
  let mut candidate_indexes = Vec::new();
  let mut embedding_inputs = Vec::new();

  for next_unit_index in 1..units.len() {
    let previous_messages = count_messages(&units[..next_unit_index]);
    let tail_messages = total_messages.saturating_sub(previous_messages);
    if previous_messages < BOUNDARY_PLANNER_MIN_SEGMENT_MESSAGES
      || tail_messages < BOUNDARY_PLANNER_MIN_SEGMENT_MESSAGES
      || starts_with_question_unit(units, next_unit_index)
    {
      continue;
    }

    candidate_indexes.push(next_unit_index);
    let (left_text, right_text) = embedding_boundary_context_texts(units, next_unit_index);
    embedding_inputs.push(left_text);
    embedding_inputs.push(right_text);
  }

  if embedding_inputs.is_empty() {
    return Ok(Vec::new());
  }

  let embeddings = embed_many(&embedding_inputs).await?;
  let mut hints = Vec::new();
  for (candidate_offset, &next_unit_index) in candidate_indexes.iter().enumerate() {
    let left = &embeddings[candidate_offset * 2];
    let right = &embeddings[candidate_offset * 2 + 1];
    let similarity = cosine_similarity(left.as_slice(), right.as_slice());
    let embedding_drop = (1.0 - similarity).clamp(0.0, 1.0);
    if embedding_drop < BOUNDARY_PLANNER_EMBEDDING_DROP_THRESHOLD {
      continue;
    }

    let mut candidate = candidate_boundary_for_index(units, boundary_context, next_unit_index);
    candidate.semantic_drop = candidate.semantic_drop.max(embedding_drop);
    candidate.score = candidate.score.max(0.95 + embedding_drop * 1.2);
    candidate.online_surprise_prior = candidate.online_surprise_prior.max(embedding_drop * 0.25);
    hints.push(candidate);
  }

  hints = space_planner_hints(hints);
  hints.truncate(8);
  hints.sort_by_key(|candidate| candidate.next_unit_index);
  Ok(hints)
}

fn embedding_boundary_context_texts(
  units: &[AnalysisUnit],
  next_unit_index: usize,
) -> (String, String) {
  let left_start = next_unit_index.saturating_sub(4);
  let right_end = (next_unit_index + 4).min(units.len());
  (
    aggregate_window_text(&units[left_start..next_unit_index]),
    aggregate_window_text(&units[next_unit_index..right_end]),
  )
}

fn space_planner_hints(mut hints: Vec<CandidateBoundary>) -> Vec<CandidateBoundary> {
  if hints.is_empty() {
    return hints;
  }

  let mut filtered: Vec<CandidateBoundary> = Vec::new();
  hints.sort_by_key(|candidate| candidate.next_unit_index);

  for hint in hints {
    if let Some(previous) = filtered.last_mut() {
      let gap = hint
        .next_unit_index
        .saturating_sub(previous.next_unit_index);
      if gap < BOUNDARY_PLANNER_MIN_HINT_SPACING {
        if planner_hint_rank(&hint) > planner_hint_rank(previous) + 0.18 {
          *previous = hint;
        }
        continue;
      }
    }
    filtered.push(hint);
  }

  filtered.sort_by(|left, right| {
    planner_hint_rank(right)
      .partial_cmp(&planner_hint_rank(left))
      .unwrap_or(std::cmp::Ordering::Equal)
      .then_with(|| left.next_unit_index.cmp(&right.next_unit_index))
  });
  filtered
}

fn planner_hint_rank(candidate: &CandidateBoundary) -> f32 {
  candidate.score + 0.35 * candidate.semantic_drop - 0.45 * candidate.micro_exchange_penalty
}

fn starts_with_question_unit(units: &[AnalysisUnit], next_unit_index: usize) -> bool {
  units
    .get(next_unit_index)
    .and_then(|unit| unit.messages.first())
    .is_some_and(|record| ends_with_question_marker(&record.message.content))
}

fn normalize_planned_boundary_kind(boundary: PlannedBoundary) -> (BoundaryReason, SurpriseLevel) {
  let boundary_reason = match boundary.boundary_reason {
    BoundaryReason::TemporalGap | BoundaryReason::SessionBreak => BoundaryReason::TopicShift,
    reason => reason,
  };
  let surprise_level = match (boundary_reason, boundary.surprise_level) {
    (BoundaryReason::TopicShift | BoundaryReason::IntentShift, SurpriseLevel::ExtremelyHigh) => {
      SurpriseLevel::High
    }
    (_, surprise_level) => surprise_level,
  };

  (boundary_reason, surprise_level)
}

fn merge_refined_boundaries(
  refined_candidates: Vec<RefinedBoundary>,
  planned_boundaries: Vec<RefinedBoundary>,
) -> Vec<RefinedBoundary> {
  let mut by_index: BTreeMap<usize, RefinedBoundary> = BTreeMap::new();

  for boundary in refined_candidates.into_iter().chain(planned_boundaries) {
    by_index
      .entry(boundary.next_unit_index)
      .and_modify(|existing| {
        if boundary_strength(&boundary) > boundary_strength(existing) {
          *existing = boundary.clone();
        }
      })
      .or_insert(boundary);
  }

  by_index.into_values().collect()
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

  let mut boundaries = stabilize_refined_boundaries(units, refined_boundaries, eof_seen);
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

fn stabilize_refined_boundaries(
  units: &[AnalysisUnit],
  refined_boundaries: &[RefinedBoundary],
  eof_seen: bool,
) -> Vec<RefinedBoundary> {
  let mut stabilized = Vec::new();
  let mut segment_start_index = 0usize;

  for boundary in refined_boundaries.iter().cloned() {
    let previous_slice = &units[segment_start_index..boundary.next_unit_index];
    let observed_tail = &units[boundary.next_unit_index..];
    if should_close_boundary(previous_slice, observed_tail, &boundary, eof_seen) {
      segment_start_index = boundary.next_unit_index;
      stabilized.push(boundary);
    }
  }

  stabilized
}

fn should_close_boundary(
  previous_slice: &[AnalysisUnit],
  observed_tail: &[AnalysisUnit],
  boundary: &RefinedBoundary,
  eof_seen: bool,
) -> bool {
  if previous_slice.is_empty() || observed_tail.is_empty() {
    return false;
  }

  if is_force_close_boundary(boundary) {
    return true;
  }

  let previous_messages = count_messages(previous_slice);
  let previous_units = previous_slice.len();
  let observed_tail_messages = count_messages(observed_tail);
  let boundary_strength = boundary_strength(boundary);

  let (min_messages, min_units, min_strength) = match boundary.boundary_reason {
    BoundaryReason::TopicShift => (
      BOUNDARY_PLANNER_MIN_SEGMENT_MESSAGES,
      BOUNDARY_PLANNER_MIN_SEGMENT_MESSAGES,
      MIN_TOPIC_SHIFT_BOUNDARY_STRENGTH,
    ),
    BoundaryReason::IntentShift => (
      BOUNDARY_PLANNER_MIN_SEGMENT_MESSAGES,
      BOUNDARY_PLANNER_MIN_SEGMENT_MESSAGES,
      MIN_INTENT_SHIFT_BOUNDARY_STRENGTH,
    ),
    BoundaryReason::SurpriseShift => (
      MIN_SURPRISE_SHIFT_SEGMENT_MESSAGES,
      MIN_SURPRISE_SHIFT_SEGMENT_UNITS,
      MIN_SURPRISE_SHIFT_BOUNDARY_STRENGTH,
    ),
    BoundaryReason::TemporalGap | BoundaryReason::SessionBreak => (1, 1, 0.0),
  };

  if previous_messages < min_messages || previous_units < min_units {
    return false;
  }

  if !eof_seen && observed_tail_messages < MIN_OBSERVED_TAIL_MESSAGES_FOR_NON_TEMPORAL_CLOSE {
    return false;
  }

  boundary_strength >= min_strength
}

fn is_force_close_boundary(boundary: &RefinedBoundary) -> bool {
  matches!(
    boundary.boundary_reason,
    BoundaryReason::TemporalGap | BoundaryReason::SessionBreak
  ) || boundary.time_gap >= 0.8
    || boundary.surprise_level == SurpriseLevel::ExtremelyHigh
}

fn boundary_strength(boundary: &RefinedBoundary) -> f32 {
  let reason_bonus = match boundary.boundary_reason {
    BoundaryReason::TemporalGap => 0.45,
    BoundaryReason::SessionBreak => 0.4,
    BoundaryReason::SurpriseShift => 0.28,
    BoundaryReason::TopicShift => 0.08,
    BoundaryReason::IntentShift => 0.02,
  };
  let surprise_bonus = match boundary.surprise_level {
    SurpriseLevel::Low => 0.0,
    SurpriseLevel::High => 0.18,
    SurpriseLevel::ExtremelyHigh => 0.4,
  };

  (boundary.candidate_score
    + reason_bonus
    + surprise_bonus
    + 0.18 * boundary.semantic_drop
    + 0.12 * boundary.online_surprise_prior
    - 0.15 * boundary.micro_exchange_penalty)
    .max(0.0)
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
  absorb_pre_gap_trailing_singletons(&mut spans);

  let mut index = 0usize;
  while index < spans.len() {
    if !should_merge_short_span(&spans, index) {
      index += 1;
      continue;
    }

    let can_merge_into_previous = index > 0;
    let can_merge_into_next =
      index + 1 < spans.len() && !is_strong_start_boundary(&spans[index + 1]);

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

fn absorb_pre_gap_trailing_singletons(spans: &mut Vec<ClosedSpan>) {
  let mut index = 1usize;
  while index + 1 < spans.len() {
    if !should_absorb_pre_gap_trailing_singleton(spans, index) {
      index += 1;
      continue;
    }

    let previous_affinity = span_affinity(&spans[index - 1], &spans[index]);
    let next_affinity = span_affinity(&spans[index], &spans[index + 1]);
    let timing_supports_absorb = has_pre_gap_trailing_reply_timing(spans, index);
    if !timing_supports_absorb
      && (previous_affinity < PRE_GAP_TRAILING_REPLY_AFFINITY_THRESHOLD
        || previous_affinity < next_affinity + PRE_GAP_TRAILING_REPLY_AFFINITY_MARGIN)
    {
      index += 1;
      continue;
    }

    let trailing = spans.remove(index);
    spans[index - 1].end_seq = trailing.end_seq;
    spans[index - 1].messages.extend(trailing.messages);
  }
}

fn should_absorb_pre_gap_trailing_singleton(spans: &[ClosedSpan], index: usize) -> bool {
  let span = &spans[index];
  let next = &spans[index + 1];

  if span.messages.len() != 1 || !is_strong_start_boundary(next) {
    return false;
  }

  if !matches!(
    span.boundary_reason,
    BoundaryReason::SessionBreak | BoundaryReason::TopicShift | BoundaryReason::IntentShift
  ) || span.surprise_level != SurpriseLevel::Low
  {
    return false;
  }

  let message = &span.messages[0];
  let content = message.content.trim();
  !content.is_empty()
    && content.len() <= PRE_GAP_TRAILING_REPLY_MAX_CHARS
    && !ends_with_question_marker(content)
}

fn has_pre_gap_trailing_reply_timing(spans: &[ClosedSpan], index: usize) -> bool {
  let Some(previous_at) = spans[index - 1]
    .messages
    .last()
    .map(|message| message.timestamp)
  else {
    return false;
  };
  let Some(current_at) = spans[index]
    .messages
    .last()
    .map(|message| message.timestamp)
  else {
    return false;
  };
  let Some(next_at) = spans[index + 1]
    .messages
    .first()
    .map(|message| message.timestamp)
  else {
    return false;
  };

  let previous_gap_minutes = (current_at - previous_at).num_minutes();
  let next_gap_minutes = (next_at - current_at).num_minutes();
  previous_gap_minutes >= 0
    && previous_gap_minutes <= PRE_GAP_TRAILING_REPLY_PREVIOUS_MAX_MINUTES
    && next_gap_minutes >= PRE_GAP_TRAILING_REPLY_NEXT_MIN_MINUTES
}

fn should_merge_short_span(spans: &[ClosedSpan], index: usize) -> bool {
  let span = &spans[index];
  span.messages.len() <= 2
    && matches!(
      span.boundary_reason,
      BoundaryReason::TopicShift | BoundaryReason::IntentShift
    )
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

pub fn build_projection_text(span: &ClosedSpan) -> (String, String) {
  (
    build_provisional_title(span),
    build_provisional_content(span),
  )
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
    lines.push(format!(
      "* {} said {}",
      message.role,
      message.content.trim()
    ));
  }

  lines.join("\n")
}

pub fn build_boundary_context(span: &ClosedSpan, title: &str) -> SegmentationBoundaryContext {
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
  for token in messages
    .iter()
    .flat_map(|message| tokenize(&message.content).into_iter())
  {
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

fn count_messages(units: &[AnalysisUnit]) -> usize {
  units.iter().map(|unit| unit.messages.len()).sum()
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
  if union <= 0.0 { 0.0 } else { overlap / union }
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

#[cfg(any(test, feature = "segmentation-debug"))]
fn debug_message_preview(message: &Message) -> String {
  let content = message.content.trim();
  let content = if content.chars().count() > 160 {
    format!("{}...", content.chars().take(160).collect::<String>())
  } else {
    content.to_owned()
  };

  format!("{}: {}", message.role, content)
}

#[cfg(test)]
mod tests;
