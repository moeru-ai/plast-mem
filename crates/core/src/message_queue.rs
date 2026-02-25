use anyhow::anyhow;
use chrono::{TimeDelta, Utc};
use plastmem_entities::message_queue;
use plastmem_shared::{AppError, Message};

use sea_orm::{
  ColumnTrait, ConnectionTrait, DatabaseConnection, DbBackend, EntityTrait, FromQueryResult,
  QueryFilter, QuerySelect, Set, Statement, TransactionTrait,
  prelude::Expr,
  sea_query::OnConflict,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ──────────────────────────────────────────────────
// Trigger constants
// ──────────────────────────────────────────────────

const MIN_MESSAGES: usize = 5;
const WINDOW_BASE: usize = 20;
const WINDOW_MAX: usize = 40;
const FENCE_TTL_MINUTES: i64 = 120;
const SOFT_TIME_TRIGGER_HOURS: i64 = 2;

// ──────────────────────────────────────────────────
// Types
// ──────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MessageQueue {
  pub id: Uuid,
  pub messages: Vec<Message>,
}

/// A pending review record from a single retrieval.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PendingReview {
  pub query: String,
  pub memory_ids: Vec<Uuid>,
}

/// Result of checking if event segmentation is needed.
#[derive(Debug, Clone)]
pub struct SegmentationCheck {
  pub fence_count: i32,
}

#[derive(Debug, FromQueryResult)]
struct PushResult {
  msg_count: i32,
}

#[derive(Debug, FromQueryResult)]
struct IdRow {
  #[allow(dead_code)]
  id: uuid::Uuid,
}

// ──────────────────────────────────────────────────
// Core queue operations
// ──────────────────────────────────────────────────

impl MessageQueue {
  pub async fn get(id: Uuid, db: &DatabaseConnection) -> Result<Self, AppError> {
    let model = Self::get_or_create_model(id, db).await?;
    Self::from_model(model)
  }

  pub async fn get_or_create_model(
    id: Uuid,
    db: &DatabaseConnection,
  ) -> Result<message_queue::Model, AppError> {
    if let Some(model) = message_queue::Entity::find_by_id(id).one(db).await? {
      return Ok(model);
    }

    let active_model = message_queue::ActiveModel {
      id: Set(id),
      messages: Set(serde_json::to_value(Vec::<Message>::new())?),
      pending_reviews: Set(None),
      in_progress_fence: Set(None),
      in_progress_since: Set(None),
      window_doubled: Set(false),
      prev_episode_summary: Set(None),
    };

    message_queue::Entity::insert(active_model)
      .on_conflict(
        OnConflict::column(message_queue::Column::Id)
          .do_nothing()
          .to_owned(),
      )
      .exec_without_returning(db)
      .await?;

    message_queue::Entity::find_by_id(id)
      .one(db)
      .await?
      .ok_or_else(|| anyhow!("Failed to ensure queue existence").into())
  }

  pub fn from_model(model: message_queue::Model) -> Result<Self, AppError> {
    Ok(Self {
      id: model.id,
      messages: serde_json::from_value(model.messages)?,
    })
  }

  /// Push a message to the queue, then check if batch segmentation should be triggered.
  pub async fn push(
    id: Uuid,
    message: Message,
    db: &DatabaseConnection,
  ) -> Result<Option<SegmentationCheck>, AppError> {
    Self::get_or_create_model(id, db).await?;

    let message_json = serde_json::to_value(vec![&message])?;
    let sql = "UPDATE message_queue \
               SET messages = messages || $1::jsonb \
               WHERE id = $2 \
               RETURNING jsonb_array_length(messages) AS msg_count";

    let result = PushResult::find_by_statement(Statement::from_sql_and_values(
      DbBackend::Postgres,
      sql,
      [message_json.into(), id.into()],
    ))
    .one(db)
    .await?;

    let trigger_count = result
      .ok_or_else(|| AppError::from(anyhow!("Queue not found after push")))?
      .msg_count;

    Self::check(id, trigger_count, db).await
  }

  /// Atomically removes the first `count` messages from the queue.
  pub async fn drain(id: Uuid, count: usize, db: &DatabaseConnection) -> Result<(), AppError> {
    let sql = format!(
      "UPDATE message_queue SET messages = jsonb_path_query_array(messages, '$[{count} to last]'::jsonpath) WHERE id = $1"
    );
    let res = db
      .execute_raw(Statement::from_sql_and_values(
        DbBackend::Postgres,
        &sql,
        [id.into()],
      ))
      .await?;

    if res.rows_affected() == 0 {
      return Err(anyhow!("Queue not found").into());
    }

    Ok(())
  }

  // ──────────────────────────────────────────────────
  // Segmentation trigger check
  // ──────────────────────────────────────────────────

  pub async fn check(
    id: Uuid,
    trigger_count: i32,
    db: &DatabaseConnection,
  ) -> Result<Option<SegmentationCheck>, AppError> {
    let model = MessageQueue::get_or_create_model(id, db).await?;

    if model.in_progress_fence.is_some() {
      let cleared = MessageQueue::clear_stale_fence(id, FENCE_TTL_MINUTES, db).await?;
      if !cleared {
        tracing::debug!(conversation_id = %id, "Segmentation skipped: job in progress");
        return Ok(None);
      }
    }

    let trigger_count_usize = trigger_count as usize;
    if trigger_count_usize < MIN_MESSAGES {
      return Ok(None);
    }

    let current_window = if model.window_doubled { WINDOW_MAX } else { WINDOW_BASE };
    let count_trigger = trigger_count_usize >= current_window;
    let messages: Vec<plastmem_shared::Message> = serde_json::from_value(model.messages)?;
    let time_trigger = messages.first().is_some_and(|first| {
      Utc::now() - first.timestamp > TimeDelta::hours(SOFT_TIME_TRIGGER_HOURS)
    });

    if !count_trigger && !time_trigger {
      return Ok(None);
    }

    if !MessageQueue::try_set_fence(id, trigger_count, db).await? {
      return Ok(None);
    }

    tracing::debug!(
      conversation_id = %id,
      trigger_count,
      count_trigger,
      time_trigger,
      window_doubled = model.window_doubled,
      "Segmentation triggered"
    );

    Ok(Some(SegmentationCheck { fence_count: trigger_count }))
  }

  // ──────────────────────────────────────────────────
  // Fence / state management
  // ──────────────────────────────────────────────────

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

  pub async fn finalize_job(
    id: Uuid,
    prev_episode_summary: Option<String>,
    db: &DatabaseConnection,
  ) -> Result<(), AppError> {
    message_queue::Entity::update_many()
      .col_expr(message_queue::Column::InProgressFence, Expr::value(Option::<i32>::None))
      .col_expr(message_queue::Column::InProgressSince, Expr::value(Option::<chrono::DateTime<chrono::FixedOffset>>::None))
      .col_expr(message_queue::Column::WindowDoubled, Expr::value(false))
      .col_expr(message_queue::Column::PrevEpisodeSummary, Expr::value(prev_episode_summary))
      .filter(message_queue::Column::Id.eq(id))
      .exec(db)
      .await?;
    Ok(())
  }

  pub async fn set_doubled_and_clear_fence(
    id: Uuid,
    db: &DatabaseConnection,
  ) -> Result<(), AppError> {
    message_queue::Entity::update_many()
      .col_expr(message_queue::Column::InProgressFence, Expr::value(Option::<i32>::None))
      .col_expr(message_queue::Column::InProgressSince, Expr::value(Option::<chrono::DateTime<chrono::FixedOffset>>::None))
      .col_expr(message_queue::Column::WindowDoubled, Expr::value(true))
      .filter(message_queue::Column::Id.eq(id))
      .exec(db)
      .await?;
    Ok(())
  }

  pub async fn get_prev_episode_summary(
    id: Uuid,
    db: &DatabaseConnection,
  ) -> Result<Option<String>, AppError> {
    let model = Self::get_or_create_model(id, db).await?;
    Ok(model.prev_episode_summary)
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
