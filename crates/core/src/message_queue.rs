use anyhow::anyhow;
use plastmem_entities::message_queue;
use plastmem_shared::AppError;

use sea_orm::{
  ConnectionTrait, DatabaseConnection, DbBackend, EntityTrait, QuerySelect, Set, Statement,
  TransactionTrait, sea_query::OnConflict,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy)]
pub struct MessageQueue;

/// A pending review record from a single retrieval.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PendingReview {
  pub query: String,
  pub memory_ids: Vec<Uuid>,
}

impl MessageQueue {
  async fn ensure_exists(id: Uuid, db: &DatabaseConnection) -> Result<(), AppError> {
    let active_model = message_queue::ActiveModel {
      id: Set(id),
      pending_reviews: Set(None),
    };

    message_queue::Entity::insert(active_model)
      .on_conflict(
        OnConflict::column(message_queue::Column::Id)
          .do_nothing()
          .to_owned(),
      )
      .exec_without_returning(db)
      .await?;

    Ok(())
  }

  // ──────────────────────────────────────────────────
  // Pending reviews
  // ──────────────────────────────────────────────────

  pub async fn add_pending_review(
    id: Uuid,
    memory_ids: Vec<Uuid>,
    query: String,
    db: &DatabaseConnection,
  ) -> Result<(), AppError> {
    Self::ensure_exists(id, db).await?;

    let review = PendingReview { query, memory_ids };
    let review_value = serde_json::to_value(vec![review])?;

    let res = db
      .execute_raw(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "UPDATE message_queue SET pending_reviews = COALESCE(pending_reviews, '[]'::jsonb) || $1::jsonb WHERE id = $2",
        [review_value.into(), id.into()],
      ))
      .await?;

    if res.rows_affected() == 0 {
      return Err(anyhow!("Queue not found").into());
    }

    Ok(())
  }

  pub async fn take_pending_reviews(
    id: Uuid,
    db: &DatabaseConnection,
  ) -> Result<Option<Vec<PendingReview>>, AppError> {
    let txn = db.begin().await?;

    let Some(model) = message_queue::Entity::find_by_id(id)
      .lock_exclusive()
      .one(&txn)
      .await?
    else {
      return Ok(None);
    };

    let reviews: Option<Vec<PendingReview>> = model
      .pending_reviews
      .and_then(|v| serde_json::from_value(v).ok())
      .filter(|v: &Vec<PendingReview>| !v.is_empty());

    if reviews.is_some() {
      message_queue::Entity::update(message_queue::ActiveModel {
        id: Set(id),
        pending_reviews: Set(None),
        ..Default::default()
      })
      .exec(&txn)
      .await?;
    }

    txn.commit().await?;

    Ok(reviews)
  }
}
