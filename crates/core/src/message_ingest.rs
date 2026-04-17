use plastmem_entities::{conversation_message, segmentation_state};
use plastmem_shared::{AppError, Message};
use sea_orm::{
  ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter,
  QueryOrder, QuerySelect, Set, TransactionTrait,
};
use uuid::Uuid;

use crate::ConversationMessage;
use crate::segmentation_state::{
  SegmentJobState, SegmentationJobClaim, SegmentationState, ensure_segmentation_state_exists,
  recover_stale_segmentation_job,
};

pub const SEGMENTATION_PENDING_TRIGGER_COUNT: i64 = 20;
pub const SEGMENTATION_GAP_MINUTES: i64 = 30;

// ──────────────────────────────────────────────────
// Public API
// ──────────────────────────────────────────────────

/// Append a single message to a conversation and attempt to claim a segmentation job.
pub async fn append_message(
  conversation_id: Uuid,
  message: Message,
  eof_identified: bool,
  db: &DatabaseConnection,
) -> Result<Option<SegmentationJobClaim>, AppError> {
  ingest_messages(conversation_id, &[message], eof_identified, db).await
}

/// Append a batch of messages to a conversation (marks eof) and attempt to claim a segmentation job.
pub async fn append_batch_messages(
  conversation_id: Uuid,
  messages: &[Message],
  db: &DatabaseConnection,
) -> Result<Option<SegmentationJobClaim>, AppError> {
  ingest_messages(conversation_id, messages, true, db).await
}

/// Recover any stale lease, then attempt to claim a segmentation job for this conversation.
///
/// Returns `Some(claim)` when the trigger conditions are met (enough pending messages,
/// a time gap between messages, or eof was identified) and the caller should enqueue
/// a segmentation job.
pub async fn try_claim_segmentation_job(
  conversation_id: Uuid,
  db: &DatabaseConnection,
) -> Result<Option<SegmentationJobClaim>, AppError> {
  recover_stale_segmentation_job(conversation_id, db).await?;
  claim_segmentation_job_if_ready(conversation_id, db).await
}

// ──────────────────────────────────────────────────
// Message ingestion
// ──────────────────────────────────────────────────

async fn ingest_messages(
  conversation_id: Uuid,
  messages: &[Message],
  eof_identified: bool,
  db: &DatabaseConnection,
) -> Result<Option<SegmentationJobClaim>, AppError> {
  if messages.is_empty() {
    return try_claim_segmentation_job(conversation_id, db).await;
  }

  let txn = db.begin().await?;
  ensure_segmentation_state_exists(conversation_id, &txn).await?;

  let state_model = segmentation_state::Entity::find_by_id(conversation_id)
    .lock_exclusive()
    .one(&txn)
    .await?
    .ok_or_else(|| AppError::new(anyhow::anyhow!("Segmentation state not found after ensure")))?;

  let start_seq = state_model.last_message_seq + 1;
  let active_models: Vec<conversation_message::ActiveModel> = messages
    .iter()
    .enumerate()
    .map(|(idx, message)| {
      ConversationMessage::from_message(conversation_id, start_seq + idx as i64, message)
        .map(|message| message.to_model())
        .map(Into::into)
    })
    .collect::<Result<_, _>>()?;

  conversation_message::Entity::insert_many(active_models)
    .exec(&txn)
    .await?;

  let last_message_seq = start_seq + i64::try_from(messages.len()).unwrap_or(0) - 1;
  let existing_eof_identified = state_model.eof_identified;
  let mut active_model: segmentation_state::ActiveModel = state_model.into();
  active_model.last_message_seq = Set(last_message_seq);
  active_model.eof_identified = Set(existing_eof_identified || eof_identified);
  active_model.update(&txn).await?;

  txn.commit().await?;

  try_claim_segmentation_job(conversation_id, db).await
}

// ──────────────────────────────────────────────────
// Trigger evaluation
// ──────────────────────────────────────────────────

/// Pure claim logic — assumes stale leases have already been recovered.
///
/// Returns `None` if the job state is Active (another worker holds the lease)
/// or the trigger conditions are not yet met.
async fn claim_segmentation_job_if_ready(
  conversation_id: Uuid,
  db: &DatabaseConnection,
) -> Result<Option<SegmentationJobClaim>, AppError> {
  let txn = db.begin().await?;
  ensure_segmentation_state_exists(conversation_id, &txn).await?;

  let state_model = segmentation_state::Entity::find_by_id(conversation_id)
    .lock_exclusive()
    .one(&txn)
    .await?
    .ok_or_else(|| AppError::new(anyhow::anyhow!("Segmentation state not found after ensure")))?;

  let state = SegmentationState::from_model(state_model.clone())?;

  let SegmentJobState::Inactive {
    next_segment_start_seq,
  } = state.job_state
  else {
    txn.commit().await?;
    return Ok(None);
  };

  if state.last_message_seq < next_segment_start_seq {
    txn.commit().await?;
    return Ok(None);
  }

  let pending_message_count = state.last_message_seq - next_segment_start_seq + 1;
  let time_gap_observed = has_pending_time_gap(
    conversation_id,
    next_segment_start_seq,
    state.last_message_seq,
    &txn,
  )
  .await?;

  if !state.eof_identified
    && pending_message_count < SEGMENTATION_PENDING_TRIGGER_COUNT
    && !time_gap_observed
  {
    txn.commit().await?;
    return Ok(None);
  }

  let claim = SegmentationJobClaim {
    conversation_id,
    active_segment_start_seq: next_segment_start_seq,
    active_segment_end_seq: state.last_message_seq,
  };
  let active_state = SegmentationState {
    conversation_id,
    last_message_seq: state.last_message_seq,
    eof_identified: state.eof_identified,
    job_state: SegmentJobState::Active {
      active_segment_start_seq: claim.active_segment_start_seq,
      active_segment_end_seq: claim.active_segment_end_seq,
      active_since: chrono::Utc::now(),
    },
  };
  let model = active_state.to_model();
  let mut active_model: segmentation_state::ActiveModel = state_model.into();
  active_model.next_segment_start_seq = Set(model.next_segment_start_seq);
  active_model.active_segment_start_seq = Set(model.active_segment_start_seq);
  active_model.active_segment_end_seq = Set(model.active_segment_end_seq);
  active_model.active_since = Set(model.active_since);
  active_model.update(&txn).await?;

  txn.commit().await?;
  Ok(Some(claim))
}

/// Check whether any adjacent pair of pending messages has a time gap ≥ `SEGMENTATION_GAP_MINUTES`.
async fn has_pending_time_gap<C>(
  conversation_id: Uuid,
  start_seq: i64,
  end_seq: i64,
  db: &C,
) -> Result<bool, AppError>
where
  C: ConnectionTrait,
{
  if end_seq - start_seq < 1 {
    return Ok(false);
  }

  let models = conversation_message::Entity::find()
    .filter(conversation_message::Column::ConversationId.eq(conversation_id))
    .filter(conversation_message::Column::Seq.gte(start_seq))
    .filter(conversation_message::Column::Seq.lte(end_seq))
    .order_by_asc(conversation_message::Column::Seq)
    .all(db)
    .await?;

  Ok(models.windows(2).any(|pair| {
    pair[1].timestamp.signed_duration_since(pair[0].timestamp)
      >= chrono::TimeDelta::minutes(SEGMENTATION_GAP_MINUTES)
  }))
}
