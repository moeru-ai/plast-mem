use anyhow::anyhow;
use plastmem_entities::message_queue;
use plastmem_shared::AppError;

use sea_orm::{
  ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QuerySelect, TransactionTrait,
  prelude::Expr,
};
use uuid::Uuid;

use super::{MessageQueue, PendingReview};

impl MessageQueue {
  /// Append a pending review record to the queue.
  /// Called after retrieve_memory to track which memories were retrieved.
  pub async fn add_pending_review(
    id: Uuid,
    memory_ids: Vec<Uuid>,
    query: String,
    db: &DatabaseConnection,
  ) -> Result<(), AppError> {
    // Ensure the queue row exists
    Self::get_or_create_model(id, db).await?;

    let review = PendingReview { query, memory_ids };
    let review_value = serde_json::to_value(vec![review])?;

    let res = message_queue::Entity::update_many()
      .col_expr(
        message_queue::Column::PendingReviews,
        Expr::cust_with_values(
          "COALESCE(pending_reviews, '[]'::jsonb) || ?::jsonb",
          [review_value],
        ),
      )
      .filter(message_queue::Column::Id.eq(id))
      .exec(db)
      .await?;

    if res.rows_affected == 0 {
      return Err(anyhow!("Queue not found").into());
    }

    Ok(())
  }

  /// Atomically take all pending reviews and clear them.
  /// Uses SELECT FOR UPDATE within a transaction to prevent race conditions.
  /// Returns the pending reviews if any, or None.
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
      message_queue::Entity::update_many()
        .col_expr(
          message_queue::Column::PendingReviews,
          Expr::value(Option::<serde_json::Value>::None),
        )
        .filter(message_queue::Column::Id.eq(id))
        .exec(&txn)
        .await?;
    }

    txn.commit().await?;

    Ok(reviews)
  }
}
