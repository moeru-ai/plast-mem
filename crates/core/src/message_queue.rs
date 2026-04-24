use anyhow::anyhow;
use chrono::TimeDelta;
use plastmem_entities::message_queue;
use plastmem_shared::{AppError, Message};

use sea_orm::{
  ConnectionTrait, DatabaseConnection, DbBackend, EntityTrait, FromQueryResult, QuerySelect, Set,
  Statement, TransactionTrait, sea_query::OnConflict,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const WINDOW_BASE: usize = 20;
const WINDOW_MAX: usize = 30;
pub const FENCE_TTL_MINUTES: i64 = 120;
const GAP_TRIGGER_HOURS: i64 = 3;

pub const ADD_BACKPRESSURE_LIMIT: i32 = 25;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MessageQueue {
  pub id: Uuid,
  pub messages: Vec<Message>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PendingReview {
  pub query: String,
  pub memory_ids: Vec<Uuid>,
}

#[derive(Debug, Clone)]
pub struct SegmentationCheck {
  pub fence_count: i32,
  pub force_process: bool,
}

#[derive(Debug, Clone)]
pub struct QueueProcessingStatus {
  pub messages_pending: i32,
  pub fence_active: bool,
}

#[derive(Debug, FromQueryResult)]
struct PushResult {
  msg_count: i32,
}

#[derive(Debug, FromQueryResult)]
struct ProcessingStatusRow {
  messages_pending: i32,
  fence_active: bool,
}

#[derive(Debug, FromQueryResult)]
struct IdRow {
  #[allow(dead_code)]
  id: Uuid,
}

impl MessageQueue {
  pub async fn get_processing_status(
    id: Uuid,
    db: &DatabaseConnection,
  ) -> Result<QueueProcessingStatus, AppError> {
    Self::ensure_exists(id, db).await?;

    let sql = "SELECT \
               COALESCE(jsonb_array_length(messages), 0)::int AS messages_pending, \
               (in_progress_fence IS NOT NULL) AS fence_active \
               FROM message_queue \
               WHERE id = $1";

    let row = ProcessingStatusRow::find_by_statement(Statement::from_sql_and_values(
      DbBackend::Postgres,
      sql,
      [id.into()],
    ))
    .one(db)
    .await?
    .ok_or_else(|| anyhow!("Queue not found after ensure_exists"))?;

    Ok(QueueProcessingStatus {
      messages_pending: row.messages_pending,
      fence_active: row.fence_active,
    })
  }

  pub async fn get(id: Uuid, db: &DatabaseConnection) -> Result<Self, AppError> {
    let model = Self::get_or_create_model(id, db).await?;
    Self::from_model(model)
  }

  async fn ensure_exists(id: Uuid, db: &DatabaseConnection) -> Result<(), AppError> {
    message_queue::Entity::insert(message_queue::ActiveModel {
      id: Set(id),
      messages: Set(serde_json::to_value(Vec::<Message>::new())?),
      pending_reviews: Set(None),
      in_progress_fence: Set(None),
      in_progress_since: Set(None),
      prev_episode_content: Set(None),
    })
    .on_conflict(
      OnConflict::column(message_queue::Column::Id)
        .do_nothing()
        .to_owned(),
    )
    .exec_without_returning(db)
    .await?;

    Ok(())
  }

  pub async fn get_or_create_model(
    id: Uuid,
    db: &DatabaseConnection,
  ) -> Result<message_queue::Model, AppError> {
    if let Some(model) = message_queue::Entity::find_by_id(id).one(db).await? {
      return Ok(model);
    }

    Self::ensure_exists(id, db).await?;

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

  pub async fn push(
    id: Uuid,
    message: Message,
    db: &DatabaseConnection,
  ) -> Result<Option<SegmentationCheck>, AppError> {
    Self::ensure_exists(id, db).await?;

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

  pub async fn push_batch(
    id: Uuid,
    messages: &[Message],
    db: &DatabaseConnection,
  ) -> Result<i32, AppError> {
    Self::ensure_exists(id, db).await?;
    if messages.is_empty() {
      let queue = Self::get(id, db).await?;
      return Ok(i32::try_from(queue.messages.len()).unwrap_or(i32::MAX));
    }

    let message_json = serde_json::to_value(messages)?;
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

    Ok(
      result
        .ok_or_else(|| AppError::from(anyhow!("Queue not found after push_batch")))?
        .msg_count,
    )
  }

  pub async fn drain<C>(id: Uuid, count: usize, db: &C) -> Result<(), AppError>
  where
    C: ConnectionTrait,
  {
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

  pub async fn check(
    id: Uuid,
    trigger_count: i32,
    db: &DatabaseConnection,
  ) -> Result<Option<SegmentationCheck>, AppError> {
    let model = Self::get_or_create_model(id, db).await?;

    if model.in_progress_fence.is_some() {
      let cleared = Self::clear_stale_fence(id, FENCE_TTL_MINUTES, db).await?;
      if !cleared {
        tracing::debug!(conversation_id = %id, "Segmentation skipped: job in progress");
        return Ok(None);
      }
    }

    let messages: Vec<Message> = serde_json::from_value(model.messages)?;
    let trigger_count_usize = usize::try_from(trigger_count).unwrap_or(0);
    let gap_trigger = messages.len() >= 2
      && messages.windows(2).last().is_some_and(|pair| {
        pair[1].timestamp - pair[0].timestamp >= TimeDelta::hours(GAP_TRIGGER_HOURS)
      });
    let count_trigger = trigger_count_usize >= WINDOW_BASE;
    let force_trigger = gap_trigger || trigger_count_usize >= WINDOW_MAX;

    if !gap_trigger && !count_trigger {
      return Ok(None);
    }

    let fence_count = if gap_trigger {
      trigger_count.saturating_sub(1)
    } else {
      trigger_count
    };

    if fence_count <= 0 || !Self::try_set_fence(id, fence_count, db).await? {
      return Ok(None);
    }

    Ok(Some(SegmentationCheck {
      fence_count,
      force_process: force_trigger,
    }))
  }

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

  pub async fn finalize_job<C>(
    id: Uuid,
    prev_episode_content: Option<String>,
    db: &C,
  ) -> Result<(), AppError>
  where
    C: ConnectionTrait,
  {
    message_queue::Entity::update(message_queue::ActiveModel {
      id: Set(id),
      in_progress_fence: Set(None),
      in_progress_since: Set(None),
      prev_episode_content: Set(prev_episode_content),
      ..Default::default()
    })
    .exec(db)
    .await?;
    Ok(())
  }

  pub async fn clear_fence(id: Uuid, db: &DatabaseConnection) -> Result<(), AppError> {
    message_queue::Entity::update(message_queue::ActiveModel {
      id: Set(id),
      in_progress_fence: Set(None),
      in_progress_since: Set(None),
      ..Default::default()
    })
    .exec(db)
    .await?;
    Ok(())
  }

  pub async fn get_prev_episode_content(
    id: Uuid,
    db: &DatabaseConnection,
  ) -> Result<Option<String>, AppError> {
    let model = Self::get_or_create_model(id, db).await?;
    Ok(model.prev_episode_content)
  }

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
      txn.commit().await?;
      return Ok(None);
    };

    let reviews: Option<Vec<PendingReview>> = model
      .pending_reviews
      .and_then(|value| serde_json::from_value(value).ok())
      .filter(|items: &Vec<PendingReview>| !items.is_empty());

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
