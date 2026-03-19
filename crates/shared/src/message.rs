use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(transparent)]
#[schema(value_type = String)]
pub struct MessageRole(pub String);

impl From<&str> for MessageRole {
  fn from(value: &str) -> Self {
    Self(value.to_owned())
  }
}

impl From<String> for MessageRole {
  fn from(value: String) -> Self {
    Self(value)
  }
}

impl fmt::Display for MessageRole {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "{}", self.0)
  }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
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
