use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MessageEvent {
  pub id: Uuid,
  pub role: MessageEventRole,
  pub content: String,
  pub timestamp: DateTime<Utc>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MessageEventRole {
  User,
  Assistant,
  Custom(String),
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Event {
  #[serde(rename = "message")]
  MessageEvent(MessageEvent),
}

impl Event {
  pub fn id(&self) -> Uuid {
    match self {
      Event::MessageEvent(e) => e.id,
    }
  }

  pub fn timestamp(&self) -> DateTime<Utc> {
    match self {
      Event::MessageEvent(e) => e.timestamp,
    }
  }
}
