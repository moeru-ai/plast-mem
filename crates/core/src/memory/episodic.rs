use crate::{Message, MessageRole};
use chrono::{DateTime, Utc};
use plast_mem_db_schema::episodic_memory;
use plast_mem_llm::{InputMessage, Role, embed, summarize_messages};
use plast_mem_shared::AppError;
use sea_orm::prelude::PgVector;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EpisodicMemory {
  pub id: Uuid,
  pub conversation_id: Uuid,
  pub messages: Vec<Message>,
  pub content: String,
  pub embedding: PgVector,
  pub start_at: DateTime<Utc>,
  pub end_at: DateTime<Utc>,
  pub created_at: DateTime<Utc>,
  pub last_reviewed_at: DateTime<Utc>,
}

impl EpisodicMemory {
  pub async fn new(conversation_id: Uuid, messages: Vec<Message>) -> Result<Self, AppError> {
    let now = Utc::now();
    let id = Uuid::now_v7();
    let start_at = messages.first().map(|m| m.timestamp).unwrap_or(now);
    let end_at = messages.last().map(|m| m.timestamp).unwrap_or(now);

    let input_messages = messages
      .iter()
      .map(|m| InputMessage {
        role: match m.role {
          MessageRole::User => Role::User,
          MessageRole::Assistant => Role::Assistant,
        },
        content: m.content.clone(),
      })
      .collect::<Vec<_>>();

    let content = summarize_messages(&input_messages).await?;
    let embedding = embed(&content).await?;

    Ok(Self {
      id,
      conversation_id,
      messages,
      content,
      embedding,
      start_at,
      end_at,
      created_at: now.clone(),
      last_reviewed_at: now.clone(),
    })
  }

  pub fn to_model(&self) -> Result<episodic_memory::Model, AppError> {
    Ok(episodic_memory::Model {
      id: self.id,
      conversation_id: self.conversation_id,
      messages: serde_json::to_value(self.messages.clone())?,
      content: self.content.clone(),
      embedding: self.embedding.clone(),
      start_at: self.start_at.into(),
      end_at: self.end_at.into(),
      created_at: self.created_at.into(),
      last_reviewed_at: self.last_reviewed_at.into(),
    })
  }
}
