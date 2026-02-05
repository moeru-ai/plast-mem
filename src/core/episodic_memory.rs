use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{MemoryState, Message, MessageRole};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EpisodicMemory {
  pub id: Uuid,
  pub messages: Vec<Message>,
  pub created_at: DateTime<Utc>,
  pub fsrs: MemoryState,
  pub search_text: String,
}

impl EpisodicMemory {
  pub fn new(messages: Vec<Message>) -> Self {
    let now = Utc::now();
    let search_text = format_messages_without_date(&messages);

    Self {
      id: Uuid::now_v7(),
      messages,
      created_at: now,
      fsrs: MemoryState::new_default(now),
      search_text,
    }
  }
}

pub fn format_messages_without_date(messages: &[Message]) -> String {
  let mut out = String::new();
  for (i, message) in messages.iter().enumerate() {
    if i > 0 {
      out.push('\n');
    }
    out.push_str(role_label(&message.role));
    out.push_str(": ");
    out.push_str(&message.content);
  }
  out
}

pub fn format_messages_with_date(messages: &[Message]) -> String {
  let mut out = String::new();
  for (i, message) in messages.iter().enumerate() {
    if i > 0 {
      out.push('\n');
    }
    out.push('[');
    out.push_str(&message.timestamp.to_rfc3339());
    out.push_str("] ");
    out.push_str(role_label(&message.role));
    out.push_str(": ");
    out.push_str(&message.content);
  }
  out
}

fn role_label(role: &MessageRole) -> &'static str {
  match role {
    MessageRole::User => "User",
    MessageRole::Assistant => "Assistant",
  }
}
