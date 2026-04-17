use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use strum::Display;

use super::EventDataToString;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MessageEventData {
  pub role: MessageEventRole,
  pub content: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Display)]
#[serde(rename_all = "snake_case")]
pub enum MessageEventRole {
  User,
  Assistant,
  Custom(String),
}

impl EventDataToString for MessageEventData {
  fn to_string_without_timestamp(&self) -> String {
    format!("{}: {}", self.role, self.content)
  }

  fn to_string_with_timestamp(&self, timestamp: DateTime<Utc>) -> String {
    format!(
      "[{}] {}",
      timestamp.format("%Y-%m-%d %H:%M:%S"),
      self.to_string_without_timestamp()
    )
  }
}
