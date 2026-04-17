use chrono::{DateTime, Utc};
use plastmem_entities::{
  EpisodeClassification, conversation_message, episode_span, segmentation_state,
};
use plastmem_shared::AppError;
use sea_orm::{
  ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter,
  QueryOrder, QuerySelect, Set, TransactionTrait, sea_query::OnConflict,
};
use serde::Serialize;
use uuid::Uuid;

use crate::ConversationMessage;

pub const SEGMENTATION_LEASE_TTL_MINUTES: i64 = 120;

// ──────────────────────────────────────────────────
// Types
// ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct SegmentationJobClaim {
  pub conversation_id: Uuid,
  pub active_segment_start_seq: i64,
  pub active_segment_end_seq: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SegmentationProcessingStatus {
  pub pending_message_count: i64,
  pub active: bool,
}

#[derive(Debug, Clone, Serialize)]
pub enum SegmentJobState {
  Inactive {
    next_segment_start_seq: i64,
  },
  Active {
    active_segment_start_seq: i64,
    active_segment_end_seq: i64,
    active_since: DateTime<Utc>,
  },
}

#[derive(Debug, Clone, Serialize)]
pub struct SegmentationState {
  pub conversation_id: Uuid,
  pub last_message_seq: i64,
  pub eof_identified: bool,
  pub job_state: SegmentJobState,
}

impl SegmentationState {
  pub fn processing_status(&self) -> SegmentationProcessingStatus {
    let next_segment_start_seq = match self.job_state {
      SegmentJobState::Inactive {
        next_segment_start_seq,
      } => next_segment_start_seq,
      SegmentJobState::Active {
        active_segment_start_seq,
        ..
      } => active_segment_start_seq,
    };

    let pending_message_count = if self.last_message_seq >= next_segment_start_seq {
      self.last_message_seq - next_segment_start_seq + 1
    } else {
      0
    };

    SegmentationProcessingStatus {
      pending_message_count,
      active: matches!(self.job_state, SegmentJobState::Active { .. }),
    }
  }

  pub fn from_model(model: segmentation_state::Model) -> Result<Self, AppError> {
    let job_state = match (
      model.active_segment_start_seq,
      model.active_segment_end_seq,
      model.active_since,
    ) {
      (Some(start), Some(end), Some(since)) => SegmentJobState::Active {
        active_segment_start_seq: start,
        active_segment_end_seq: end,
        active_since: since.with_timezone(&Utc),
      },
      (None, None, None) => SegmentJobState::Inactive {
        next_segment_start_seq: model.next_segment_start_seq,
      },
      _ => {
        return Err(AppError::new(anyhow::anyhow!(
          "Invalid segmentation_state row: active fields are partially populated"
        )));
      }
    };

    Ok(Self {
      conversation_id: model.conversation_id,
      last_message_seq: model.last_message_seq,
      eof_identified: model.eof_identified,
      job_state,
    })
  }

  pub fn to_model(&self) -> segmentation_state::Model {
    let (next_segment_start_seq, active_segment_start_seq, active_segment_end_seq, active_since) =
      match &self.job_state {
        SegmentJobState::Inactive {
          next_segment_start_seq,
        } => (*next_segment_start_seq, None, None, None),
        SegmentJobState::Active {
          active_segment_start_seq,
          active_segment_end_seq,
          active_since,
        } => (
          *active_segment_start_seq,
          Some(*active_segment_start_seq),
          Some(*active_segment_end_seq),
          Some((*active_since).into()),
        ),
      };

    segmentation_state::Model {
      conversation_id: self.conversation_id,
      last_message_seq: self.last_message_seq,
      eof_identified: self.eof_identified,
      next_segment_start_seq,
      active_segment_start_seq,
      active_segment_end_seq,
      active_since,
    }
  }
}

#[derive(Debug, Clone, Serialize)]
pub struct EpisodeSpan {
  pub conversation_id: Uuid,
  pub start_seq: i64,
  pub end_seq: i64,
  pub classification: EpisodeClassification,
  pub created_at: DateTime<Utc>,
}

impl EpisodeSpan {
  pub fn from_model(model: episode_span::Model) -> Self {
    Self {
      conversation_id: model.conversation_id,
      start_seq: model.start_seq,
      end_seq: model.end_seq,
      classification: model.classification,
      created_at: model.created_at.with_timezone(&Utc),
    }
  }

  pub fn to_model(&self) -> episode_span::Model {
    episode_span::Model {
      conversation_id: self.conversation_id,
      start_seq: self.start_seq,
      end_seq: self.end_seq,
      classification: self.classification.clone(),
      created_at: self.created_at.into(),
    }
  }
}

// ──────────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────────

pub(crate) async fn ensure_segmentation_state_exists<C>(
  conversation_id: Uuid,
  db: &C,
) -> Result<(), AppError>
where
  C: ConnectionTrait,
{
  segmentation_state::Entity::insert(segmentation_state::ActiveModel {
    conversation_id: Set(conversation_id),
    last_message_seq: Set(-1),
    eof_identified: Set(false),
    next_segment_start_seq: Set(0),
    active_segment_start_seq: Set(None),
    active_segment_end_seq: Set(None),
    active_since: Set(None),
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

/// Clear the active lease fields on a segmentation state active model,
/// transitioning it back to Inactive.
fn clear_active_fields(active_model: &mut segmentation_state::ActiveModel) {
  active_model.active_segment_start_seq = Set(None);
  active_model.active_segment_end_seq = Set(None);
  active_model.active_since = Set(None);
}

async fn load_state_for_update<C>(
  conversation_id: Uuid,
  db: &C,
) -> Result<(segmentation_state::Model, SegmentationState), AppError>
where
  C: ConnectionTrait,
{
  ensure_segmentation_state_exists(conversation_id, db).await?;
  let model = segmentation_state::Entity::find_by_id(conversation_id)
    .lock_exclusive()
    .one(db)
    .await?
    .ok_or_else(|| AppError::new(anyhow::anyhow!("Segmentation state not found after ensure")))?;
  let state = SegmentationState::from_model(model.clone())?;
  Ok((model, state))
}

// ──────────────────────────────────────────────────
// Read operations
// ──────────────────────────────────────────────────

pub async fn get_segmentation_state(
  conversation_id: Uuid,
  db: &DatabaseConnection,
) -> Result<SegmentationState, AppError> {
  let model = segmentation_state::Entity::find_by_id(conversation_id)
    .one(db)
    .await?
    .ok_or_else(|| AppError::new(anyhow::anyhow!("Segmentation state not found")))?;
  SegmentationState::from_model(model)
}

pub async fn get_segmentation_processing_status(
  conversation_id: Uuid,
  db: &DatabaseConnection,
) -> Result<SegmentationProcessingStatus, AppError> {
  let state = get_segmentation_state(conversation_id, db).await?;
  Ok(state.processing_status())
}

pub async fn get_claim_messages(
  claim: &SegmentationJobClaim,
  db: &DatabaseConnection,
) -> Result<Vec<ConversationMessage>, AppError> {
  get_messages_in_range(
    claim.conversation_id,
    claim.active_segment_start_seq,
    claim.active_segment_end_seq,
    db,
  )
  .await
}

pub async fn get_messages_in_range(
  conversation_id: Uuid,
  start_seq: i64,
  end_seq: i64,
  db: &DatabaseConnection,
) -> Result<Vec<ConversationMessage>, AppError> {
  let models = conversation_message::Entity::find()
    .filter(conversation_message::Column::ConversationId.eq(conversation_id))
    .filter(conversation_message::Column::Seq.gte(start_seq))
    .filter(conversation_message::Column::Seq.lte(end_seq))
    .order_by_asc(conversation_message::Column::Seq)
    .all(db)
    .await?;

  Ok(
    models
      .into_iter()
      .map(ConversationMessage::from_model)
      .collect(),
  )
}

pub async fn get_episode_span(
  conversation_id: Uuid,
  start_seq: i64,
  db: &DatabaseConnection,
) -> Result<Option<EpisodeSpan>, AppError> {
  episode_span::Entity::find_by_id((conversation_id, start_seq))
    .one(db)
    .await?
    .map(|model| Ok(EpisodeSpan::from_model(model)))
    .transpose()
}

// ──────────────────────────────────────────────────
// Job lifecycle operations
// ──────────────────────────────────────────────────

pub async fn recover_stale_segmentation_job(
  conversation_id: Uuid,
  db: &DatabaseConnection,
) -> Result<bool, AppError> {
  let txn = db.begin().await?;

  let (state_model, state) = load_state_for_update(conversation_id, &txn).await?;
  let SegmentJobState::Active {
    active_segment_start_seq,
    active_since,
    ..
  } = state.job_state
  else {
    txn.commit().await?;
    return Ok(false);
  };

  let expired_before = Utc::now() - chrono::TimeDelta::minutes(SEGMENTATION_LEASE_TTL_MINUTES);
  if active_since >= expired_before {
    txn.commit().await?;
    return Ok(false);
  }

  let mut active_model: segmentation_state::ActiveModel = state_model.into();
  active_model.next_segment_start_seq = Set(active_segment_start_seq);
  clear_active_fields(&mut active_model);
  active_model.update(&txn).await?;

  txn.commit().await?;
  Ok(true)
}

pub async fn commit_segmentation_job(
  claim: &SegmentationJobClaim,
  finalized_spans: &[EpisodeSpan],
  next_segment_start_seq: i64,
  db: &DatabaseConnection,
) -> Result<(), AppError> {
  let txn = db.begin().await?;

  let (state_model, state) = load_state_for_update(claim.conversation_id, &txn).await?;
  match state.job_state {
    SegmentJobState::Active {
      active_segment_start_seq,
      active_segment_end_seq,
      ..
    } if active_segment_start_seq == claim.active_segment_start_seq
      && active_segment_end_seq == claim.active_segment_end_seq => {}
    _ => {
      return Err(AppError::new(anyhow::anyhow!(
        "Cannot commit segmentation job: active claim mismatch"
      )));
    }
  }

  if next_segment_start_seq < claim.active_segment_start_seq
    || next_segment_start_seq > claim.active_segment_end_seq + 1
  {
    return Err(AppError::new(anyhow::anyhow!(
      "Invalid next_segment_start_seq for committed claim"
    )));
  }

  if !finalized_spans.is_empty() {
    let active_models: Vec<episode_span::ActiveModel> = finalized_spans
      .iter()
      .map(EpisodeSpan::to_model)
      .map(Into::into)
      .collect();
    episode_span::Entity::insert_many(active_models)
      .exec(&txn)
      .await?;
  }

  let mut active_model: segmentation_state::ActiveModel = state_model.into();
  active_model.eof_identified = Set(false);
  active_model.next_segment_start_seq = Set(next_segment_start_seq);
  clear_active_fields(&mut active_model);
  active_model.update(&txn).await?;

  txn.commit().await?;
  Ok(())
}

pub async fn abort_segmentation_job(
  claim: &SegmentationJobClaim,
  db: &DatabaseConnection,
) -> Result<(), AppError> {
  let txn = db.begin().await?;

  let (state_model, state) = load_state_for_update(claim.conversation_id, &txn).await?;
  match state.job_state {
    SegmentJobState::Active {
      active_segment_start_seq,
      active_segment_end_seq,
      ..
    } if active_segment_start_seq == claim.active_segment_start_seq
      && active_segment_end_seq == claim.active_segment_end_seq => {}
    _ => {
      txn.commit().await?;
      return Ok(());
    }
  }

  let mut active_model: segmentation_state::ActiveModel = state_model.into();
  active_model.next_segment_start_seq = Set(claim.active_segment_start_seq);
  clear_active_fields(&mut active_model);
  active_model.update(&txn).await?;

  txn.commit().await?;
  Ok(())
}
