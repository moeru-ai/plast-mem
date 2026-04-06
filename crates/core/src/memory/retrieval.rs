use std::fmt::Write;

use chrono::Utc;
use chrono_humanize::HumanTime;
use serde::Deserialize;
use utoipa::ToSchema;

use super::EpisodicMemory;
use super::SemanticMemory;

fn format_message_timestamp(message_time: chrono::DateTime<Utc>) -> String {
  message_time.format("%Y-%m-%d %H:%M UTC").to_string()
}

#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum DetailLevel {
  /// Ranks 1-2 always include original message details
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
      Self::Auto => rank <= 2,
      Self::None => false,
      Self::Low => rank == 1 && surprise >= 0.7,
      Self::High => true,
    }
  }
}

#[must_use]
pub fn format_tool_result(
  semantic_results: &[(SemanticMemory, f64)],
  episodic_results: &[(EpisodicMemory, f64)],
  detail: &DetailLevel,
) -> String {
  let mut out = String::new();

  // ── Episodic Memories ──
  if !episodic_results.is_empty() {
    let _ = writeln!(out, "## Episodic Memories");
  }

  let now = Utc::now();

  for (rank, (mem, _score)) in episodic_results.iter().enumerate() {
    let rank = rank + 1; // 1-indexed

    // Header
    let header = if mem.title.is_empty() {
      format!("Memory {rank}")
    } else {
      mem.title.clone()
    };
    let _ = writeln!(out, "### {header}");

    // When
    let relative = HumanTime::from(mem.end_at.signed_duration_since(now));
    let absolute = mem.end_at.format("%Y-%m-%d %H:%M UTC");
    let _ = writeln!(out, "**Conversation Time:** {absolute} ({relative})");

    // Content
    let _ = writeln!(out, "**Content:** {}", mem.content);

    // Details
    if detail.include_details(rank, mem.surprise) {
      let _ = writeln!(out, "\n**Details:**");
      for msg in &mem.messages {
        let _ = writeln!(
          out,
          "- [{}] {}: \"{}\"",
          format_message_timestamp(msg.timestamp),
          msg.role,
          msg.content,
        );
      }
    }

    let _ = writeln!(out);
  }

  // ── Known Facts ──
  if !semantic_results.is_empty() {
    let _ = writeln!(out, "## Known Facts");
    for (fact, _score) in semantic_results {
      let _ = writeln!(out, "- {}", fact.fact);
    }
    let _ = writeln!(out);
  }

  out.trim_end().to_string()
}
