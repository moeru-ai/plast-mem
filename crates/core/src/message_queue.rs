use anyhow::anyhow;
use chrono::TimeDelta;
use plastmem_entities::message_queue;
use plastmem_shared::AppError;

use sea_orm::{
  ColumnTrait, DatabaseConnection, EntityTrait, ExprTrait, QueryFilter, QuerySelect, Set,
  TransactionTrait,
  prelude::{Expr, PgVector},
  sea_query::{BinOper, OnConflict, extension::postgres::PgBinOper},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Message;

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

  /// Check if event segmentation should be triggered.
  /// Returns `Ok(Some(SegmentationCheck))` if segmentation is needed.
  pub async fn check(
    id: Uuid,
    message: &Message,
    db: &DatabaseConnection,
  ) -> Result<Option<SegmentationCheck>, AppError> {
    let messages = Self::get(id, db).await?.messages;

    // === Hard rules (no LLM call) ===

    // Too few messages: never segment.
    if messages.len() < 3 {
      return Ok(None);
    }

    // Buffer full: force split (drain all messages).
    if messages.len() >= 30 {
      return Ok(Some(SegmentationCheck {
        messages,
        action: SegmentationAction::ForceCreate,
      }));
    }

    // Time gap exceeded: boundary (keep last message for next event).
    // Use the configured minutes threshold.
    if messages.last().is_some_and(|last_message| {
      message.timestamp - last_message.timestamp > TimeDelta::minutes(15)
    }) {
      return Ok(Some(SegmentationCheck {
        messages,
        action: SegmentationAction::TimeBoundary,
      }));
    }

    // === Content quality checks ===

    // Total character budget too low — not enough content to segment.
    let total_chars: usize = messages.iter().map(|m| m.content.chars().count()).sum();
    // Use configured minimum characters.
    if total_chars < 100 {
      return Ok(None);
    }

    // Latest message too short to trigger a boundary evaluation.
    if message.content.chars().count() < 5 {
      return Ok(None);
    }

    // === Passed rules → needs LLM boundary detection ===
    Ok(Some(SegmentationCheck {
      messages,
      action: SegmentationAction::NeedsBoundaryDetection,
    }))
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

  /// Update the event model description for a conversation.
  pub async fn update_event_model(
    id: Uuid,
    event_model: Option<String>,
    db: &DatabaseConnection,
  ) -> Result<(), AppError> {
    message_queue::Entity::update_many()
      .col_expr(message_queue::Column::EventModel, Expr::value(event_model))
      .filter(message_queue::Column::Id.eq(id))
      .exec(db)
      .await?;

    Ok(())
  }

  /// Update the last embedding for cosine similarity pre-filtering.
  pub async fn update_last_embedding(
    id: Uuid,
    embedding: Option<PgVector>,
    db: &DatabaseConnection,
  ) -> Result<(), AppError> {
    message_queue::Entity::update_many()
      .col_expr(message_queue::Column::LastEmbedding, Expr::value(embedding))
      .filter(message_queue::Column::Id.eq(id))
      .exec(db)
      .await?;

    Ok(())
  }

  /// Get the current event model for a conversation.
  pub async fn get_event_model(
    id: Uuid,
    db: &DatabaseConnection,
  ) -> Result<Option<String>, AppError> {
    let model = Self::get_or_create_model(id, db).await?;
    Ok(model.event_model)
  }

  /// Get the last embedding for a conversation.
  pub async fn get_last_embedding(
    id: Uuid,
    db: &DatabaseConnection,
  ) -> Result<Option<PgVector>, AppError> {
    let model = Self::get_or_create_model(id, db).await?;
    Ok(model.last_embedding)
  }
}
