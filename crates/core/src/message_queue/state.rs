use anyhow::anyhow;
use plastmem_entities::message_queue;
use plastmem_shared::AppError;

use sea_orm::{
  ColumnTrait, ConnectionTrait, DatabaseConnection, DbBackend, EntityTrait, FromQueryResult,
  QueryFilter, QuerySelect, Statement, TransactionTrait,
  prelude::Expr,
};
use uuid::Uuid;

use super::{MessageQueue, PendingReview};

#[derive(Debug, FromQueryResult)]
struct IdRow {
  id: uuid::Uuid,
}

impl MessageQueue {
  /// Attempt to atomically set the in-progress fence to `fence_count`.
  ///
  /// `fence_count` is the exact message count captured at push time (via RETURNING), so the
  /// fence boundary is pinned to the message that triggered this check regardless of any
  /// concurrent pushes that may have occurred since.
  ///
  /// Returns `true` if the lock was acquired, `false` if another job already holds it.
  pub async fn try_set_fence(
    id: Uuid,
    fence_count: i32,
    db: &DatabaseConnection,
  ) -> Result<bool, AppError> {
    let sql = "UPDATE message_queue \
               SET in_progress_fence = $2, in_progress_since = NOW() \
               WHERE id = $1 AND in_progress_fence IS NULL \
               RETURNING id";

    let result = IdRow::find_by_statement(Statement::from_sql_and_values(
      DbBackend::Postgres,
      sql,
      [id.into(), fence_count.into()],
    ))
    .one(db)
    .await?;

    Ok(result.is_some())
  }

  /// Clear any fence that has exceeded the TTL (stale job recovery).
  /// Returns true if a stale fence was cleared.
  pub async fn clear_stale_fence(
    id: Uuid,
    ttl_minutes: i64,
    db: &DatabaseConnection,
  ) -> Result<bool, AppError> {
    let sql = "UPDATE message_queue \
      SET in_progress_fence = NULL, in_progress_since = NULL \
      WHERE id = $1 \
        AND in_progress_fence IS NOT NULL \
        AND in_progress_since < NOW() - ($2 || ' minutes')::INTERVAL \
      RETURNING id";

    let result = IdRow::find_by_statement(Statement::from_sql_and_values(
      DbBackend::Postgres,
      sql,
      [id.into(), ttl_minutes.to_string().into()],
    ))
    .one(db)
    .await?;

    Ok(result.is_some())
  }

  /// Clear fence + reset window_doubled + update prev_episode_summary after a successful drain.
  pub async fn finalize_job(
    id: Uuid,
    prev_episode_summary: Option<String>,
    db: &DatabaseConnection,
  ) -> Result<(), AppError> {
    let fence: Option<i32> = None;
    let since: Option<chrono::DateTime<chrono::FixedOffset>> = None;

    message_queue::Entity::update_many()
      .col_expr(message_queue::Column::InProgressFence, Expr::value(fence))
      .col_expr(message_queue::Column::InProgressSince, Expr::value(since))
      .col_expr(message_queue::Column::WindowDoubled, Expr::value(false))
      .col_expr(
        message_queue::Column::PrevEpisodeSummary,
        Expr::value(prev_episode_summary),
      )
      .filter(message_queue::Column::Id.eq(id))
      .exec(db)
      .await?;

    Ok(())
  }

  /// Clear fence + set window_doubled = true (no-split path, waiting for more messages).
  pub async fn set_doubled_and_clear_fence(
    id: Uuid,
    db: &DatabaseConnection,
  ) -> Result<(), AppError> {
    let fence: Option<i32> = None;
    let since: Option<chrono::DateTime<chrono::FixedOffset>> = None;

    message_queue::Entity::update_many()
      .col_expr(message_queue::Column::InProgressFence, Expr::value(fence))
      .col_expr(message_queue::Column::InProgressSince, Expr::value(since))
      .col_expr(message_queue::Column::WindowDoubled, Expr::value(true))
      .filter(message_queue::Column::Id.eq(id))
      .exec(db)
      .await?;

    Ok(())
  }

  /// Get the summary of the last drained episode.
  pub async fn get_prev_episode_summary(
    id: Uuid,
    db: &DatabaseConnection,
  ) -> Result<Option<String>, AppError> {
    let model = Self::get_or_create_model(id, db).await?;
    Ok(model.prev_episode_summary)
  }

  /// Append a pending review record to the queue.
  /// Called after `retrieve_memory` to track which memories were retrieved.
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
