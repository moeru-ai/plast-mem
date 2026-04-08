use anyhow::anyhow;
use apalis::prelude::{Data, TaskSink};
use apalis_postgres::PostgresStorage;
use chrono::Utc;
use fsrs::{DEFAULT_PARAMETERS, FSRS};
use plastmem_ai::embed;
use plastmem_core::{
  ConversationMessageRecord, MessageQueue, SEGMENTATION_WINDOW_BASE, get_segmentation_state,
  list_messages,
};
use plastmem_entities::{episode_span, episodic_memory, segmentation_state};
use plastmem_event_segmentation::{
  AnalysisUnit, BoundaryReason, ClosedSpan, SurpriseLevel, build_boundary_context,
  build_projection_text, segment_records,
};
#[cfg(feature = "segmentation-debug")]
pub use plastmem_event_segmentation::{
  DebugBoundaryTrace, DebugSegmentationMessage, DebugSegmentationMode, DebugSegmentationSpan,
  DebugSegmentationTrace, debug_segment_messages, debug_segment_messages_with_trace,
};
use plastmem_shared::{APP_ENV, AppError, Message};
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
const MIN_MESSAGES_FOR_IMMEDIATE_CONSOLIDATION: usize = 10;
const MIN_CHARS_FOR_IMMEDIATE_CONSOLIDATION: usize = 700;
const MIN_MESSAGES_FOR_HIGH_SURPRISE_CONSOLIDATION: usize = 6;
const MIN_CHARS_FOR_HIGH_SURPRISE_CONSOLIDATION: usize = 420;
const MIN_MESSAGES_FOR_STRONG_BREAK_CONSOLIDATION: usize = 3;
const MIN_CHARS_FOR_STRONG_BREAK_CONSOLIDATION: usize = 180;
const MIN_DURATION_MINUTES_FOR_IMMEDIATE_CONSOLIDATION: i64 = 120;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventSegmentationJob {
  pub conversation_id: Uuid,
}

#[derive(Debug, Clone)]
struct CreatedEpisode {
  id: Uuid,
  surprise: f32,
  message_count: usize,
  total_chars: usize,
  duration_minutes: i64,
  boundary_reason: BoundaryReason,
  surprise_level: SurpriseLevel,
}

struct PreparedClosedSpan {
  span_id: Uuid,
  start_seq: i64,
  end_seq: i64,
  boundary_reason: BoundaryReason,
  surprise_level: SurpriseLevel,
  message_count: usize,
  total_chars: usize,
  episodic_memory: plastmem_core::EpisodicMemory,
  boundary_context: plastmem_core::SegmentationBoundaryContext,
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

  let read_start_seq = state
    .open_tail_start_seq
    .unwrap_or(state.next_unsegmented_seq);
  if read_start_seq > in_progress_until_seq {
    finalize_empty_pass(job.conversation_id, db).await?;
    return Ok(());
  }

  let records = list_messages(
    job.conversation_id,
    read_start_seq,
    in_progress_until_seq,
    db,
  )
  .await?;
  if records.is_empty() {
    finalize_empty_pass(job.conversation_id, db).await?;
    return Ok(());
  }

  let segmentation = segment_records(
    &records,
    state.last_closed_boundary_context.as_ref(),
    state.open_tail_start_seq.is_some(),
    state.eof_seen,
  )
  .await?;

  enqueue_pending_reviews(
    job.conversation_id,
    &to_messages(&records),
    db,
    &review_storage,
  )
  .await?;
  let created = persist_closed_spans(
    job.conversation_id,
    in_progress_until_seq,
    state.eof_seen,
    &segmentation.analysis_units,
    &segmentation.closed_spans,
    db,
  )
  .await?;
  enqueue_predict_calibrate_jobs(job.conversation_id, &created, &semantic_storage).await?;
  enqueue_follow_up_if_needed(job.conversation_id, db, &segmentation_storage).await?;

  Ok(())
}

async fn persist_closed_spans(
  conversation_id: Uuid,
  claimed_until_seq: i64,
  eof_seen: bool,
  units: &[AnalysisUnit],
  closed_spans: &[ClosedSpan],
  db: &DatabaseConnection,
) -> Result<Vec<CreatedEpisode>, AppError> {
  let prepared_spans = prepare_closed_spans(conversation_id, closed_spans).await?;
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

  for prepared in prepared_spans {
    let now = Utc::now();
    episode_span::ActiveModel {
      id: Set(prepared.span_id),
      conversation_id: Set(conversation_id),
      start_seq: Set(prepared.start_seq),
      end_seq: Set(prepared.end_seq),
      boundary_reason: Set(prepared.boundary_reason.as_str().to_owned()),
      surprise_level: Set(prepared.surprise_level.as_str().to_owned()),
      status: Set("derived".to_owned()),
      created_at: Set(now.into()),
    }
    .insert(&txn)
    .await?;

    let episodic_active_model: episodic_memory::ActiveModel =
      prepared.episodic_memory.to_model()?.into();
    episodic_memory::Entity::insert(episodic_active_model)
      .exec(&txn)
      .await?;

    max_closed_end_seq = Some(prepared.end_seq);
    next_unsegmented_seq = prepared.end_seq + 1;
    last_boundary_context = Some(serde_json::to_value(prepared.boundary_context)?);
    created.push(CreatedEpisode {
      id: prepared.episodic_memory.id,
      surprise: prepared.episodic_memory.surprise,
      message_count: prepared.message_count,
      total_chars: prepared.total_chars,
      duration_minutes: (prepared.episodic_memory.end_at - prepared.episodic_memory.start_at)
        .num_minutes()
        .max(0),
      boundary_reason: prepared.boundary_reason,
      surprise_level: prepared.surprise_level,
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
    Some(
      units
        .first()
        .map_or(state_model.next_unsegmented_seq, |unit| unit.start_seq),
    )
  };

  if can_finalize_tail_now && max_closed_end_seq.is_none() {
    next_unsegmented_seq = claimed_until_seq + 1;
  }

  let should_clear_eof =
    effective_eof_seen && open_tail_start_seq.is_none() && processed_all_seen_messages;
  let mut active_state: segmentation_state::ActiveModel = state_model.into_active_model();
  active_state.next_unsegmented_seq = Set(next_unsegmented_seq);
  active_state.open_tail_start_seq = Set(open_tail_start_seq);
  active_state.in_progress_until_seq = Set(None);
  active_state.in_progress_since = Set(None);
  active_state.eof_seen = Set(if should_clear_eof {
    false
  } else {
    effective_eof_seen
  });
  active_state.last_closed_boundary_context = Set(last_boundary_context);
  active_state.updated_at = Set(Utc::now().into());
  active_state.update(&txn).await?;

  txn.commit().await?;
  Ok(created)
}

async fn prepare_closed_spans(
  conversation_id: Uuid,
  closed_spans: &[ClosedSpan],
) -> Result<Vec<PreparedClosedSpan>, AppError> {
  let mut prepared = Vec::with_capacity(closed_spans.len());

  for span in closed_spans {
    prepared.push(build_prepared_closed_span(conversation_id, span).await?);
  }

  Ok(prepared)
}

async fn build_prepared_closed_span(
  conversation_id: Uuid,
  span: &ClosedSpan,
) -> Result<PreparedClosedSpan, AppError> {
  let now = Utc::now();
  let span_id = Uuid::now_v7();
  let (title, content) = build_projection_text(span);
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
  let start_at = span
    .messages
    .first()
    .map_or(now, |message| message.timestamp);
  let end_at = span
    .messages
    .last()
    .map_or(now, |message| message.timestamp);
  let episodic_memory = plastmem_core::EpisodicMemory {
    id: Uuid::now_v7(),
    conversation_id,
    source_span_id: Some(span_id),
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
  };
  let boundary_context = build_boundary_context(span, &episodic_memory.title);

  Ok(PreparedClosedSpan {
    span_id,
    start_seq: span.start_seq,
    end_seq: span.end_seq,
    boundary_reason: span.boundary_reason,
    surprise_level: span.surprise_level,
    message_count: span.messages.len(),
    total_chars: span
      .messages
      .iter()
      .map(|message| message.content.len())
      .sum(),
    episodic_memory,
    boundary_context,
  })
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
  let mut enqueued = 0usize;
  let mut deferred = 0usize;
  for episode in episodes {
    if !should_enqueue_predict_calibrate(episode) {
      deferred += 1;
      tracing::debug!(
        conversation_id = %conversation_id,
        episode_id = %episode.id,
        message_count = episode.message_count,
        total_chars = episode.total_chars,
        duration_minutes = episode.duration_minutes,
        boundary_reason = episode.boundary_reason.as_str(),
        surprise_level = episode.surprise_level.as_str(),
        "Deferring predict-calibrate for small or weak episode"
      );
      continue;
    }

    let mut storage = semantic_storage.clone();
    storage
      .push(PredictCalibrateJob {
        conversation_id,
        episode_id: episode.id,
        force: episode.surprise >= FLASHBULB_SURPRISE_THRESHOLD,
      })
      .await?;
    enqueued += 1;
  }

  if !episodes.is_empty() {
    tracing::info!(
      conversation_id = %conversation_id,
      total_episodes = episodes.len(),
      enqueued_predict_calibrate = enqueued,
      deferred_predict_calibrate = deferred,
      "Finished predict-calibrate enqueue pass"
    );
  }
  Ok(())
}

fn should_enqueue_predict_calibrate(episode: &CreatedEpisode) -> bool {
  if episode.surprise >= FLASHBULB_SURPRISE_THRESHOLD {
    return true;
  }

  if matches!(
    episode.boundary_reason,
    BoundaryReason::TemporalGap | BoundaryReason::SessionBreak | BoundaryReason::SurpriseShift
  ) {
    return episode.message_count >= MIN_MESSAGES_FOR_STRONG_BREAK_CONSOLIDATION
      || episode.total_chars >= MIN_CHARS_FOR_STRONG_BREAK_CONSOLIDATION;
  }

  if episode.message_count >= MIN_MESSAGES_FOR_IMMEDIATE_CONSOLIDATION {
    return true;
  }

  if episode.total_chars >= MIN_CHARS_FOR_IMMEDIATE_CONSOLIDATION
    && episode.message_count >= MIN_MESSAGES_FOR_HIGH_SURPRISE_CONSOLIDATION
  {
    return true;
  }

  if episode.duration_minutes >= MIN_DURATION_MINUTES_FOR_IMMEDIATE_CONSOLIDATION
    && episode.message_count >= MIN_MESSAGES_FOR_HIGH_SURPRISE_CONSOLIDATION
  {
    return true;
  }

  episode.surprise_level == SurpriseLevel::High
    && episode.message_count >= MIN_MESSAGES_FOR_HIGH_SURPRISE_CONSOLIDATION
    && episode.total_chars >= MIN_CHARS_FOR_HIGH_SURPRISE_CONSOLIDATION
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
  storage
    .push(EventSegmentationJob { conversation_id })
    .await?;
  Ok(())
}

fn to_messages(records: &[ConversationMessageRecord]) -> Vec<Message> {
  records
    .iter()
    .map(|record| record.message.clone())
    .collect()
}

#[cfg(test)]
mod tests {
  use super::*;

  fn created_episode(
    boundary_reason: BoundaryReason,
    surprise_level: SurpriseLevel,
    message_count: usize,
    total_chars: usize,
    duration_minutes: i64,
    surprise: f32,
  ) -> CreatedEpisode {
    CreatedEpisode {
      id: Uuid::now_v7(),
      surprise,
      message_count,
      total_chars,
      duration_minutes,
      boundary_reason,
      surprise_level,
    }
  }

  #[test]
  fn predict_calibrate_gate_skips_small_low_topic_shift_episode() {
    let episode = created_episode(
      BoundaryReason::TopicShift,
      SurpriseLevel::Low,
      3,
      180,
      10,
      0.2,
    );

    assert!(!should_enqueue_predict_calibrate(&episode));
  }

  #[test]
  fn predict_calibrate_gate_keeps_large_topic_shift_episode() {
    let episode = created_episode(
      BoundaryReason::TopicShift,
      SurpriseLevel::High,
      10,
      720,
      90,
      0.6,
    );

    assert!(should_enqueue_predict_calibrate(&episode));
  }

  #[test]
  fn predict_calibrate_gate_keeps_temporal_gap_episode() {
    let episode = created_episode(
      BoundaryReason::TemporalGap,
      SurpriseLevel::Low,
      3,
      190,
      180,
      0.2,
    );

    assert!(should_enqueue_predict_calibrate(&episode));
  }
}
