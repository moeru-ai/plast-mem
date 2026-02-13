use std::fmt::Write;

use chrono::Utc;
use chrono_humanize::HumanTime;
use serde::Deserialize;

use super::EpisodicMemory;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DetailLevel {
  /// Ranks 1-2 with surprise >= 0.7 get details
  #[default]
  Auto,
  /// No details for any memory
  None,
  /// Only rank 1 gets details (if surprising)
  Low,
  /// All returned memories get full details
  High,
}

impl DetailLevel {
  fn include_details(&self, rank: usize, surprise: f32) -> bool {
    match self {
      DetailLevel::Auto => rank <= 2 && surprise >= 0.7,
      DetailLevel::None => false,
      DetailLevel::Low => rank == 1 && surprise >= 0.7,
      DetailLevel::High => true,
    }
  }
}

pub fn format_tool_result(results: &[(EpisodicMemory, f64)], detail: DetailLevel) -> String {
  let mut out = String::new();
  let now = Utc::now();

  for (rank, (mem, score)) in results.iter().enumerate() {
    let rank = rank + 1; // 1-indexed

    // Header
    let key_moment = if mem.surprise >= 0.7 {
      ", key moment"
    } else {
      ""
    };
    let _ = writeln!(
      out,
      "## Memory {rank} [rank: {rank}, score: {score:.2}{key_moment}]"
    );

    // When
    let duration = now.signed_duration_since(mem.end_at);
    let relative = HumanTime::from(duration);
    let _ = writeln!(out, "**When:** {relative}");

    // Summary
    let _ = writeln!(out, "**Summary:** {}", mem.content);

    // Details
    if detail.include_details(rank, mem.surprise) {
      let _ = writeln!(out, "\n**Details:**");
      for msg in &mem.messages {
        let _ = writeln!(out, "- {}: \"{}\"", msg.role, msg.content);
      }
    }

    let _ = writeln!(out);
  }

  out.trim_end().to_string()
}
