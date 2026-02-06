use chrono::{DateTime, Utc};

use crate::core::{EpisodicMemory, ReviewGrade, ReviewLog};

// TODO: The design of reviewer hasn't been fixed
pub struct ReviewInput<'a> {
  pub user_input: &'a str,
  pub retrieved: &'a [EpisodicMemory],
  pub llm_output: &'a str,
}

pub fn review(_input: &ReviewInput<'_>) -> Vec<ReviewLog> {
  vec![]
}

pub fn log_review(grade: ReviewGrade, now: DateTime<Utc>) -> ReviewLog {
  ReviewLog {
    timestamp: now,
    grade,
  }
}
