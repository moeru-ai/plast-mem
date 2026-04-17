use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use strum::{Display, EnumString};

use super::EventDataToString;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MessageEventData {
  pub role: MessageEventRole,
  pub content: String,
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

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Display, EnumString)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case", ascii_case_insensitive)]
pub enum MessageEventRole {
  User,
  Assistant,
  #[strum(default)]
  Custom(String),
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::str::FromStr;

  #[test]
  fn test_from_str() {
    assert_eq!(
      MessageEventRole::from_str("user").unwrap(),
      MessageEventRole::User
    );
    assert_eq!(
      MessageEventRole::from_str("assistant").unwrap(),
      MessageEventRole::Assistant
    );
    assert_eq!(
      MessageEventRole::from_str("system").unwrap(),
      MessageEventRole::Custom("system".to_string())
    );
  }
}
