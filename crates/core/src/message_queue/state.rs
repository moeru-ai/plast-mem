use plastmem_entities::message_queue;
use plastmem_shared::AppError;

use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, prelude::{Expr, PgVector}};
use uuid::Uuid;

use super::MessageQueue;

impl MessageQueue {
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

  /// Update the event model embedding for surprise channel computation.
  pub async fn update_event_model_embedding(
    id: Uuid,
    embedding: Option<PgVector>,
    db: &DatabaseConnection,
  ) -> Result<(), AppError> {
    message_queue::Entity::update_many()
      .col_expr(
        message_queue::Column::EventModelEmbedding,
        Expr::value(embedding),
      )
      .filter(message_queue::Column::Id.eq(id))
      .exec(db)
      .await?;

    Ok(())
  }

  /// Get the event model embedding for a conversation.
  pub async fn get_event_model_embedding(
    id: Uuid,
    db: &DatabaseConnection,
  ) -> Result<Option<PgVector>, AppError> {
    let model = Self::get_or_create_model(id, db).await?;
    Ok(model.event_model_embedding)
  }
}
