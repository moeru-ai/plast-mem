use crate::utils::AppError;

use super::Message;
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set, Unchanged};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use plast_mem_db_schema::message_queue;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MessageQueue {
  pub id: Uuid,
  pub messages: Vec<Message>,
}

impl MessageQueue {
  pub async fn new(id: Uuid, db: &DatabaseConnection) -> Result<Self, AppError> {
    match message_queue::Entity::find_by_id(id).one(db).await? {
      Some(model) => Self::from_model(model),
      None => Self::init(id, db).await,
    }
  }

  pub fn from_model(model: message_queue::Model) -> Result<Self, AppError> {
    Ok(Self {
      id: model.id,
      messages: serde_json::from_value(model.messages)?,
    })
  }

  pub async fn save(&mut self, db: &DatabaseConnection) -> Result<(), AppError> {
    let active_model = message_queue::ActiveModel {
      id: Unchanged(self.id),
      messages: Set(serde_json::to_value(&self.messages)?),
    };

    active_model.update(db).await?;

    Ok(())
  }

  pub async fn push(&mut self, message: Message, db: &DatabaseConnection) -> Result<(), AppError> {
    self.messages.push(message);
    self.save(db).await?;

    // TODO: check segment

    Ok(())
  }

  pub async fn init(id: Uuid, db: &DatabaseConnection) -> Result<Self, AppError> {
    let messages: Vec<Message> = vec![];

    let active_model = message_queue::ActiveModel {
      id: Set(id),
      messages: Set(serde_json::to_value(messages)?),
    };

    let model = active_model.insert(db).await?;

    Self::from_model(model)
  }
}
