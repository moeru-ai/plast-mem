use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
  User,
  Assistant,
}

impl fmt::Display for MessageRole {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      MessageRole::User => write!(f, "user"),
      MessageRole::Assistant => write!(f, "assistant"),
    }
  }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Message {
  pub role: MessageRole,
  pub content: String,
  pub timestamp: DateTime<Utc>,
}

// impl Message {
//   pub fn is_user(&self) -> bool {
//     matches!(self.role, MessageRole::User)
//   }

//   pub fn is_assistant(&self) -> bool {
//     matches!(self.role, MessageRole::Assistant)
//   }
// }
