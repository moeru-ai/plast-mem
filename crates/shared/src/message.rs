use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
  User,
  Assistant,
}

impl fmt::Display for MessageRole {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::User => write!(f, "User"),
      Self::Assistant => write!(f, "Assistant"),
    }
  }
}

#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
pub struct Message {
  pub role: MessageRole,
  pub content: String,
  pub timestamp: DateTime<Utc>,
}

impl fmt::Display for Message {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    writeln!(f, "{}: {}", self.role, self.content)
  }
}

// impl Message {
//   pub fn is_user(&self) -> bool {
//     matches!(self.role, MessageRole::User)
//   }

//   pub fn is_assistant(&self) -> bool {
//     matches!(self.role, MessageRole::Assistant)
//   }
// }
