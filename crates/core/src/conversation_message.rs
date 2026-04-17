use chrono::{DateTime, Utc};
use plastmem_entities::conversation_message;
use plastmem_shared::{AppError, MessageRole};
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
pub struct ConversationMessage {
  pub conversation_id: Uuid,
  pub seq: i64,
  pub role: MessageRole,
  pub content: String,
  pub timestamp: DateTime<Utc>,
}

impl ConversationMessage {
  pub fn from_model(model: conversation_message::Model) -> Self {
    Self {
      conversation_id: model.conversation_id,
      seq: model.seq,
      role: model.role.into(),
      content: model.content,
      timestamp: model.timestamp.with_timezone(&Utc),
    }
  }

  pub fn to_model(&self) -> conversation_message::Model {
    conversation_message::Model {
      conversation_id: self.conversation_id,
      seq: self.seq,
      role: self.role.0.clone(),
      content: self.content.clone(),
      timestamp: self.timestamp.into(),
    }
  }

  pub fn to_message(&self) -> plastmem_shared::Message {
    plastmem_shared::Message {
      role: self.role.clone(),
      content: self.content.clone(),
      timestamp: self.timestamp,
    }
  }

  pub fn from_message(
    conversation_id: Uuid,
    seq: i64,
    message: &plastmem_shared::Message,
  ) -> Result<Self, AppError> {
    Ok(Self {
      conversation_id,
      seq,
      role: message.role.clone(),
      content: message.content.clone(),
      timestamp: message.timestamp,
    })
  }
}
