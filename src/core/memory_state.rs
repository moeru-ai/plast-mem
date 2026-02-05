use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "lowercase")]
pub enum ReviewGrade {
  Again,
  Hard,
  Good,
  Easy,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ReviewLog {
  pub timestamp: DateTime<Utc>,
  pub grade: ReviewGrade,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MemoryState {
  pub stability: f32,
  pub retrievability: f32,
  pub last_review_at: DateTime<Utc>,
  pub review_history: Vec<ReviewLog>,
}

impl MemoryState {
  pub fn new_default(now: DateTime<Utc>) -> Self {
    Self {
      stability: 0.1,
      retrievability: 0.9,
      last_review_at: now,
      review_history: vec![],
    }
  }
}
