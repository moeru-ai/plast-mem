use chrono::{DateTime, Utc};
use plast_mem_db_schema::episodic_memory;
use sea_orm::prelude::PgVector;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{core::Message, utils::AppError};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EpisodicMemory {
  pub id: Uuid,
  pub conversation_id: Uuid,
  pub messages: Vec<Message>,
  pub start_at: DateTime<Utc>,
  pub end_at: DateTime<Utc>,
  pub created_at: DateTime<Utc>,
  pub last_reviewed_at: DateTime<Utc>,
}

impl EpisodicMemory {
  pub fn new(conversation_id: Uuid, messages: Vec<Message>) -> Self {
    let now = Utc::now();
    let id = Uuid::now_v7();
    let start_at = messages.first().map(|m| m.timestamp).unwrap_or(now);
    let end_at = messages.last().map(|m| m.timestamp).unwrap_or(now);

    Self {
      id,
      conversation_id,
      messages,
      start_at,
      end_at,
      created_at: now.clone(),
      last_reviewed_at: now.clone(),
    }
  }

  pub fn to_model(&self) -> Result<episodic_memory::Model, AppError> {
    // TODO: call llm to generate content
    let content = serde_json::to_string(&self.messages)?;

    // TODO: generate embedding from content
    let embedding = PgVector::from(vec![]);

    Ok(episodic_memory::Model {
      id: self.id,
      conversation_id: self.conversation_id,
      messages: serde_json::to_value(self.messages.clone())?,
      content,
      embedding,
      start_at: self.start_at.into(),
      end_at: self.end_at.into(),
      created_at: self.created_at.into(),
      last_reviewed_at: self.last_reviewed_at.into(),
    })
  }
}
