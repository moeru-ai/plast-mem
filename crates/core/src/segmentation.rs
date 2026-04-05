use anyhow::anyhow;
use chrono::{DateTime, TimeDelta, Utc};
use plastmem_entities::{conversation_message, segmentation_state};
use plastmem_shared::{AppError, Message, MessageRole};
use sea_orm::{
  ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, IntoActiveModel,
  QueryFilter, QueryOrder, QuerySelect, Set, TransactionTrait, sea_query::OnConflict,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const ADD_BACKPRESSURE_LIMIT: i64 = 25;
pub const SEGMENTATION_WINDOW_BASE: i64 = 20;
pub const SEGMENTATION_WINDOW_MAX: i64 = 30;
pub const SEGMENTATION_GAP_TRIGGER_HOURS: i64 = 3;
pub const SEGMENTATION_IN_PROGRESS_TTL_MINUTES: i64 = 120;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentationBoundaryContext {
  pub anchor_topic: String,
  pub anchor_entities: Vec<String>,
  pub last_turns_compact: Vec<String>,
  pub boundary_style_hint: String,
}

#[derive(Debug, Clone)]
pub struct SegmentationState {
  pub conversation_id: Uuid,
  pub next_message_seq: i64,
  pub next_unsegmented_seq: i64,
  pub open_tail_start_seq: Option<i64>,
  pub last_seen_seq: Option<i64>,
  pub eof_seen: bool,
  pub in_progress_until_seq: Option<i64>,
  pub in_progress_since: Option<DateTime<Utc>>,
  pub last_closed_boundary_context: Option<SegmentationBoundaryContext>,
}

#[derive(Debug, Clone)]
pub struct SegmentationProcessingStatus {
  pub messages_pending: i64,
  pub fence_active: bool,
  pub eof_seen: bool,
  pub next_unsegmented_seq: i64,
  pub last_seen_seq: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct IngestResult {
  pub inserted_count: i64,
  pub last_seen_seq: Option<i64>,
  pub job_enqueued: bool,
}

#[derive(Debug, Clone)]
pub struct ConversationMessageRecord {
  pub id: Uuid,
  pub conversation_id: Uuid,
  pub seq: i64,
  pub message: Message,
  pub created_at: DateTime<Utc>,
}

impl SegmentationState {
  pub fn from_model(model: segmentation_state::Model) -> Result<Self, AppError> {
    Ok(Self {
      conversation_id: model.conversation_id,
      next_message_seq: model.next_message_seq,
      next_unsegmented_seq: model.next_unsegmented_seq,
      open_tail_start_seq: model.open_tail_start_seq,
      last_seen_seq: model.last_seen_seq,
      eof_seen: model.eof_seen,
      in_progress_until_seq: model.in_progress_until_seq,
      in_progress_since: model.in_progress_since.map(|dt| dt.with_timezone(&Utc)),
      last_closed_boundary_context: model
        .last_closed_boundary_context
        .map(serde_json::from_value)
        .transpose()?,
    })
  }
}

pub async fn ensure_segmentation_state<C>(conversation_id: Uuid, db: &C) -> Result<(), AppError>
where
  C: ConnectionTrait,
{
  let now = Utc::now();
  segmentation_state::Entity::insert(segmentation_state::ActiveModel {
    conversation_id: Set(conversation_id),
    next_message_seq: Set(0),
    next_unsegmented_seq: Set(0),
    open_tail_start_seq: Set(None),
    last_seen_seq: Set(None),
    eof_seen: Set(false),
    in_progress_until_seq: Set(None),
    in_progress_since: Set(None),
    last_closed_boundary_context: Set(None),
    strategy_version: Set("span_v2".to_owned()),
    created_at: Set(now.into()),
    updated_at: Set(now.into()),
  })
  .on_conflict(
    OnConflict::column(segmentation_state::Column::ConversationId)
      .do_nothing()
      .to_owned(),
  )
  .exec_without_returning(db)
  .await?;

  Ok(())
}

pub async fn get_segmentation_state(
  conversation_id: Uuid,
  db: &DatabaseConnection,
) -> Result<SegmentationState, AppError> {
  ensure_segmentation_state(conversation_id, db).await?;
  let model = segmentation_state::Entity::find_by_id(conversation_id)
    .one(db)
    .await?
    .ok_or_else(|| anyhow!("Segmentation state missing after ensure"))?;
  SegmentationState::from_model(model)
}

pub async fn clear_stale_in_progress(
  conversation_id: Uuid,
  ttl_minutes: i64,
  db: &DatabaseConnection,
) -> Result<bool, AppError> {
  ensure_segmentation_state(conversation_id, db).await?;
  let txn = db.begin().await?;
  let Some(model) = segmentation_state::Entity::find_by_id(conversation_id)
    .lock_exclusive()
    .one(&txn)
    .await?
  else {
    return Err(anyhow!("Segmentation state missing after ensure").into());
  };

  let Some(in_progress_since) = model
    .in_progress_since
    .map(|dt: sea_orm::prelude::DateTimeWithTimeZone| dt.with_timezone(&Utc))
  else {
    txn.commit().await?;
    return Ok(false);
  };

  if Utc::now() - in_progress_since < TimeDelta::minutes(ttl_minutes) {
    txn.commit().await?;
    return Ok(false);
  }

  let mut active_model: segmentation_state::ActiveModel = model.into();
  active_model.in_progress_until_seq = Set(None);
  active_model.in_progress_since = Set(None);
  active_model.updated_at = Set(Utc::now().into());
  active_model.update(&txn).await?;
  txn.commit().await?;
  Ok(true)
}

pub async fn get_processing_status(
  conversation_id: Uuid,
  db: &DatabaseConnection,
) -> Result<SegmentationProcessingStatus, AppError> {
  let state = get_segmentation_state(conversation_id, db).await?;
  Ok(SegmentationProcessingStatus {
    messages_pending: pending_message_count(&state),
    fence_active: state.in_progress_until_seq.is_some(),
    eof_seen: state.eof_seen,
    next_unsegmented_seq: state.next_unsegmented_seq,
    last_seen_seq: state.last_seen_seq,
  })
}

pub async fn append_messages(
  conversation_id: Uuid,
  messages: &[Message],
  eof: bool,
  source: Option<&str>,
  import_id: Option<Uuid>,
  db: &DatabaseConnection,
) -> Result<IngestResult, AppError> {
  let txn = db.begin().await?;
  ensure_segmentation_state(conversation_id, &txn).await?;

  let Some(state_model) = segmentation_state::Entity::find_by_id(conversation_id)
    .lock_exclusive()
    .one(&txn)
    .await?
  else {
    return Err(anyhow!("Segmentation state missing after ensure").into());
  };

  let previous_last_message = match state_model.last_seen_seq {
    Some(last_seen_seq) => conversation_message::Entity::find()
      .filter(conversation_message::Column::ConversationId.eq(conversation_id))
      .filter(conversation_message::Column::Seq.eq(last_seen_seq))
      .one(&txn)
      .await?,
    None => None,
  };

  let start_seq = state_model.next_message_seq;
  let inserted_count = i64::try_from(messages.len())
    .map_err(|_| anyhow!("message count does not fit in i64"))?;
  let now = Utc::now();

  if !messages.is_empty() {
    let active_models = messages
      .iter()
      .enumerate()
      .map(|(offset, message)| {
        let seq = start_seq
          + i64::try_from(offset).map_err(|_| anyhow!("message offset does not fit in i64"))?;
        Ok(conversation_message::ActiveModel {
          id: Set(Uuid::now_v7()),
          conversation_id: Set(conversation_id),
          seq: Set(seq),
          role: Set(message.role.0.clone()),
          content: Set(message.content.clone()),
          timestamp: Set(message.timestamp.into()),
          created_at: Set(now.into()),
          source: Set(source.map(ToOwned::to_owned)),
          import_id: Set(import_id),
        })
      })
      .collect::<Result<Vec<_>, AppError>>()?;

    conversation_message::Entity::insert_many(active_models)
      .exec(&txn)
      .await?;
  }

  let last_seen_seq = if inserted_count > 0 {
    Some(start_seq + inserted_count - 1)
  } else {
    state_model.last_seen_seq
  };

  let gap_trigger = gap_triggered(
    previous_last_message.map(|m| m.timestamp.with_timezone(&Utc)),
    messages,
  );

  let next_unsegmented_seq = state_model.next_unsegmented_seq;
  let open_tail_start_seq = state_model.open_tail_start_seq;
  let last_closed_boundary_context = state_model
    .last_closed_boundary_context
    .clone()
    .map(serde_json::from_value)
    .transpose()?;
  let prior_in_progress_until_seq = state_model.in_progress_until_seq;
  let eof_seen = eof;

  let state_snapshot = SegmentationState {
    conversation_id,
    next_message_seq: start_seq + inserted_count,
    next_unsegmented_seq,
    open_tail_start_seq,
    last_seen_seq,
    eof_seen,
    in_progress_until_seq: prior_in_progress_until_seq,
    in_progress_since: state_model
      .in_progress_since
      .map(|dt: sea_orm::prelude::DateTimeWithTimeZone| dt.with_timezone(&Utc)),
    last_closed_boundary_context,
  };

  let mut next_state: segmentation_state::ActiveModel = state_model.into_active_model();
  next_state.next_message_seq = Set(start_seq + inserted_count);
  next_state.last_seen_seq = Set(last_seen_seq);
  next_state.eof_seen = Set(eof_seen);
  next_state.updated_at = Set(now.into());

  let has_processable_work =
    state_snapshot.last_seen_seq.is_some() && pending_message_count(&state_snapshot) > 0;
  let should_schedule = state_snapshot.in_progress_until_seq.is_none()
    && (gap_trigger
      || pending_message_count(&state_snapshot) >= SEGMENTATION_WINDOW_BASE
      || (state_snapshot.eof_seen
        && (has_processable_work || state_snapshot.open_tail_start_seq.is_some())));

  if should_schedule {
    next_state.in_progress_until_seq = Set(state_snapshot.last_seen_seq);
    next_state.in_progress_since = Set(Some(now.into()));
  }

  next_state.update(&txn).await?;
  txn.commit().await?;

  Ok(IngestResult {
    inserted_count,
    last_seen_seq,
    job_enqueued: should_schedule,
  })
}

pub async fn list_messages(
  conversation_id: Uuid,
  start_seq: i64,
  end_seq: i64,
  db: &DatabaseConnection,
) -> Result<Vec<ConversationMessageRecord>, AppError> {
  let models = conversation_message::Entity::find()
    .filter(conversation_message::Column::ConversationId.eq(conversation_id))
    .filter(conversation_message::Column::Seq.gte(start_seq))
    .filter(conversation_message::Column::Seq.lte(end_seq))
    .order_by_asc(conversation_message::Column::Seq)
    .all(db)
    .await?;

  Ok(models
    .into_iter()
    .map(|model| ConversationMessageRecord {
      id: model.id,
      conversation_id: model.conversation_id,
      seq: model.seq,
      message: Message {
        role: MessageRole(model.role),
        content: model.content,
        timestamp: model.timestamp.with_timezone(&Utc),
      },
      created_at: model.created_at.with_timezone(&Utc),
    })
    .collect())
}

fn pending_message_count(state: &SegmentationState) -> i64 {
  match state.last_seen_seq {
    Some(last_seen_seq) if last_seen_seq >= state.next_unsegmented_seq => {
      last_seen_seq - state.next_unsegmented_seq + 1
    }
    _ => 0,
  }
}

fn gap_triggered(previous_last_timestamp: Option<DateTime<Utc>>, messages: &[Message]) -> bool {
  let mut previous = previous_last_timestamp;
  for message in messages {
    if let Some(prev) = previous
      && message.timestamp - prev >= TimeDelta::hours(SEGMENTATION_GAP_TRIGGER_HOURS)
    {
      return true;
    }
    previous = Some(message.timestamp);
  }
  false
}
