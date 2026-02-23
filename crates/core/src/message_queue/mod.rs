mod segmentation;
mod state;
mod check;

pub use check::SegmentationCheck;
pub use segmentation::{BatchSegment, SurpriseLevel, batch_segment};

use anyhow::anyhow;
use plastmem_entities::message_queue;
use plastmem_shared::{AppError, Message};

use sea_orm::{
  ConnectionTrait, DatabaseConnection, DbBackend, EntityTrait, FromQueryResult, Set, Statement,
  sea_query::OnConflict,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

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

/// What kind of segmentation action was determined.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SegmentationAction {
  /// Window threshold or 2-hour soft trigger reached — run batch LLM segmentation.
  BatchProcess,
  /// Window was already doubled and LLM still returned 1 segment — force drain as single episode.
  ForceCreate,
}

#[derive(Debug, FromQueryResult)]
struct PushResult {
  msg_count: i32,
}

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
  ///
  /// Uses a single atomic SQL UPDATE + RETURNING to append the message and capture the exact
  /// message count at push time. This count is passed directly to `check()` as the trigger
  /// boundary, preventing later-arriving messages from being included in this batch's fence.
  ///
  /// Returns `Ok(Some(SegmentationCheck))` if a segmentation job should be created.
  pub async fn push(
    id: Uuid,
    message: Message,
    db: &DatabaseConnection,
  ) -> Result<Option<SegmentationCheck>, AppError> {
    // Ensure queue exists before pushing
    Self::get_or_create_model(id, db).await?;

    // Append message and capture the exact count after this push.
    // Wrapping message in an array matches the JSONB concat operator expectation.
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

  /// Atomically removes the first `count` messages from the queue,
  /// preserving any messages appended after the read.
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
}
