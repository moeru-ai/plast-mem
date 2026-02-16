pub mod boundary;
mod pending_reviews;
mod segmentation;
mod state;

use anyhow::anyhow;
use plastmem_entities::message_queue;
use plastmem_shared::{AppError, Message};

use sea_orm::{
  ColumnTrait, DatabaseConnection, EntityTrait, ExprTrait, QueryFilter, Set,
  prelude::Expr,
  sea_query::{BinOper, OnConflict, extension::postgres::PgBinOper},
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

/// What kind of segmentation action the rules determined.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SegmentationAction {
  /// Buffer is full — force-create an episode, drain all messages.
  ForceCreate,
  /// Time gap exceeded — create episode, but the triggering message belongs to the next event.
  TimeBoundary,
  /// Rules passed — needs LLM boundary detection (with embedding pre-filter).
  NeedsBoundaryDetection,
}

/// Result of checking if event segmentation is needed.
#[derive(Debug, Clone)]
pub struct SegmentationCheck {
  pub messages: Vec<Message>,
  pub action: SegmentationAction,
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
      event_model: Set(None),
      last_embedding: Set(None),
      event_model_embedding: Set(None),
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

  /// Push a message to the queue and check if segmentation is needed.
  /// Returns `Ok(Some(SegmentationCheck))` if a segmentation job should be created.
  pub async fn push(
    id: Uuid,
    message: Message,
    db: &DatabaseConnection,
  ) -> Result<Option<SegmentationCheck>, AppError> {
    let check_result = Self::check(id, &message, db).await?;

    let message_value = serde_json::to_value(vec![message])?;

    let res = message_queue::Entity::update_many()
      .col_expr(
        message_queue::Column::Messages,
        Expr::col(message_queue::Column::Messages).binary(
          BinOper::PgOperator(PgBinOper::Concatenate),
          Expr::val(message_value),
        ),
      )
      .filter(message_queue::Column::Id.eq(id))
      .exec(db)
      .await?;

    if res.rows_affected == 0 {
      return Err(anyhow!("Queue not found").into());
    }

    Ok(check_result)
  }

  /// Atomically removes the first `count` messages from the queue,
  /// preserving any messages appended after the read.
  pub async fn drain(id: Uuid, count: usize, db: &DatabaseConnection) -> Result<(), AppError> {
    let res = message_queue::Entity::update_many()
      .col_expr(
        message_queue::Column::Messages,
        Expr::cust_with_values(
          "jsonb_path_query_array(messages, ?::jsonpath)",
          [&format!("$[{count} to last]")],
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
}
