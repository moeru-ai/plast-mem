use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BoundaryType {
  TemporalGap,
  ContentShift,
  GoalCompletion,
  PredictionError,
}

impl BoundaryType {
  /// Retrieval boost factor based on boundary type.
  ///
  /// Applied as a multiplier to the final retrieval score:
  /// `final_score = rrf_score * retrievability * retrieval_boost`
  pub fn retrieval_boost(&self, surprise: f32) -> f64 {
    match self {
      // Highest boost: unexpected events carry high-value signals
      BoundaryType::PredictionError => 1.3 + 0.2 * surprise as f64,
      // Elevated boost: completion states summarize outcomes
      BoundaryType::GoalCompletion => 1.2,
      // Neutral: significance depends on content matching
      BoundaryType::ContentShift => 1.0,
      // Reduced boost: longer gaps imply less continuity
      BoundaryType::TemporalGap => 0.9,
    }
  }
}

impl fmt::Display for BoundaryType {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      BoundaryType::TemporalGap => write!(f, "TemporalGap"),
      BoundaryType::ContentShift => write!(f, "ContentShift"),
      BoundaryType::GoalCompletion => write!(f, "GoalCompletion"),
      BoundaryType::PredictionError => write!(f, "PredictionError"),
    }
  }
}

impl FromStr for BoundaryType {
  type Err = anyhow::Error;

  fn from_str(s: &str) -> Result<Self, Self::Err> {
    match s {
      "TemporalGap" => Ok(BoundaryType::TemporalGap),
      "ContentShift" => Ok(BoundaryType::ContentShift),
      "GoalCompletion" => Ok(BoundaryType::GoalCompletion),
      "PredictionError" => Ok(BoundaryType::PredictionError),
      _ => Err(anyhow::anyhow!("unknown boundary type: {s}")),
    }
  }
}
