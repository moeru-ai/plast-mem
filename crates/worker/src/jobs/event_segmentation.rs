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

  if starts_with_topic_cue(&next.message.content) {
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

    let cue_phrase = if starts_with_topic_cue(&current.joined_content) {
      0.7
    } else {
      0.0
    };

    let semantic_drop = semantic_drop_score(&previous.token_set, &current.token_set);
    let online_surprise_prior =
      online_surprise_prior(previous, current, boundary_context, time_gap, cue_phrase, semantic_drop);
    let score = time_gap + cue_phrase + semantic_drop + online_surprise_prior;

    if score >= 1.2 || time_gap >= 0.8 || cue_phrase >= 0.7 {
      candidates.push(CandidateBoundary {
        next_unit_index,
        score,
        time_gap,
        cue_phrase,
        semantic_drop,
        online_surprise_prior,
      });
    }
  }

  candidates
}

fn semantic_drop_score(previous: &BTreeSet<String>, current: &BTreeSet<String>) -> f32 {
  if previous.is_empty() || current.is_empty() {
    return 0.1;
  }

  let overlap = previous.iter().filter(|token| current.contains(*token)).count() as f32;
  let union = previous.union(current).count() as f32;
  (1.0 - overlap / union).clamp(0.0, 1.0)
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
    0.1
  } else {
    0.0
  };

  (0.35 * time_gap + 0.25 * cue_phrase + 0.25 * semantic_drop + 0.15 * context_novelty + speaker_shift)
    .clamp(0.0, 1.0)
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
     - online_surprise_prior: {:.2}\n\n\
     Local units:\n{}",
    candidate.time_gap,
    candidate.cue_phrase,
    candidate.semantic_drop,
    candidate.online_surprise_prior,
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
  let is_boundary = candidate.score >= 1.6 || candidate.time_gap >= 0.8 || candidate.cue_phrase >= 0.7;
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

  closed
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

  let open_tail_start_seq = if eof_seen {
    None
  } else if let Some(last_closed_end_seq) = max_closed_end_seq {
    Some(last_closed_end_seq + 1)
  } else {
    Some(units.first().map_or(state_model.next_unsegmented_seq, |unit| unit.start_seq))
  };

  if eof_seen && max_closed_end_seq.is_none() {
    next_unsegmented_seq = claimed_until_seq + 1;
  }

  let should_clear_eof = eof_seen && open_tail_start_seq.is_none();
  let mut active_state: segmentation_state::ActiveModel = state_model.into_active_model();
  active_state.next_unsegmented_seq = Set(next_unsegmented_seq);
  active_state.open_tail_start_seq = Set(open_tail_start_seq);
  active_state.in_progress_until_seq = Set(None);
  active_state.in_progress_since = Set(None);
  active_state.eof_seen = Set(if should_clear_eof { false } else { eof_seen });
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

fn truncate(text: &str, max_len: usize) -> String {
  if text.len() <= max_len {
    return text.to_owned();
  }

  let mut truncated = text.chars().take(max_len).collect::<String>();
  truncated.push_str("...");
  truncated
}

fn starts_with_topic_cue(text: &str) -> bool {
  let normalized = text.trim().to_ascii_lowercase();
  [
    "by the way",
    "anyway",
    "quick question",
    "speaking of",
    "changing topics",
    "also",
    "顺便",
    "另外",
    "话说",
    "换个话题",
  ]
  .iter()
  .any(|cue| normalized.starts_with(cue))
}

fn to_messages(records: &[ConversationMessageRecord]) -> Vec<Message> {
  records.iter().map(|record| record.message.clone()).collect()
}
