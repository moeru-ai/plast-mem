use chrono::{DateTime, Utc};
use plastmem_entities::pending_review_queue;
use sea_orm::{
  ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
  QuerySelect, Set, TransactionTrait, sea_query::Expr,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use plastmem_shared::AppError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingReview {
  pub query: String,
  pub memory_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PendingReviewQueueItem {
  pub id: Uuid,
  pub conversation_id: Uuid,
  pub query: String,
  pub memory_ids: Vec<Uuid>,
  pub created_at: DateTime<Utc>,
  pub consumed_at: Option<DateTime<Utc>>,
}

impl PendingReviewQueueItem {
  pub fn from_model(model: pending_review_queue::Model) -> Self {
    Self {
      id: model.id,
      conversation_id: model.conversation_id,
      query: model.query,
      memory_ids: model.memory_ids,
      created_at: model.created_at.with_timezone(&Utc),
      consumed_at: model.consumed_at.map(|dt| dt.with_timezone(&Utc)),
    }
  }

  pub fn to_model(&self) -> pending_review_queue::Model {
    pending_review_queue::Model {
      id: self.id,
      conversation_id: self.conversation_id,
      query: self.query.clone(),
      memory_ids: self.memory_ids.clone(),
      created_at: self.created_at.into(),
      consumed_at: self.consumed_at.map(Into::into),
    }
  }
}

pub async fn add_pending_review_item(
  conversation_id: Uuid,
  memory_ids: Vec<Uuid>,
  query: String,
  db: &DatabaseConnection,
) -> Result<(), AppError> {
  pending_review_queue::ActiveModel {
    id: Set(Uuid::now_v7()),
    conversation_id: Set(conversation_id),
    query: Set(query),
    memory_ids: Set(memory_ids),
    created_at: Set(Utc::now().into()),
    consumed_at: Set(None),
  }
  .insert(db)
  .await?;

  Ok(())
}

pub async fn take_pending_review_items(
  conversation_id: Uuid,
  db: &DatabaseConnection,
) -> Result<Option<Vec<PendingReview>>, AppError> {
  let txn = db.begin().await?;

  let models = pending_review_queue::Entity::find()
    .filter(pending_review_queue::Column::ConversationId.eq(conversation_id))
    .filter(pending_review_queue::Column::ConsumedAt.is_null())
    .order_by_asc(pending_review_queue::Column::CreatedAt)
    .lock_exclusive()
    .all(&txn)
    .await?;

  if models.is_empty() {
    txn.commit().await?;
    return Ok(None);
  }

  let ids: Vec<Uuid> = models.iter().map(|model| model.id).collect();
  let consumed_at = Utc::now();

  pending_review_queue::Entity::update_many()
    .col_expr(
      pending_review_queue::Column::ConsumedAt,
      Expr::value(consumed_at),
    )
    .filter(pending_review_queue::Column::Id.is_in(ids))
    .exec(&txn)
    .await?;

  txn.commit().await?;

  Ok(Some(
    models
      .into_iter()
      .map(|model| PendingReview {
        query: model.query,
        memory_ids: model.memory_ids,
      })
      .collect(),
  ))
}
