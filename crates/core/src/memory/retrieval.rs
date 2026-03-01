use std::fmt::Write;

use chrono::Utc;
use chrono_humanize::HumanTime;
use serde::Deserialize;
use utoipa::ToSchema;

use super::EpisodicMemory;
use super::SemanticMemory;

#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
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
      Self::Auto => rank <= 2 && surprise >= 0.7,
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

  // ── Known Facts ──
  if !semantic_results.is_empty() {
    let _ = writeln!(out, "## Known Facts");
    for (fact, _score) in semantic_results {
      let sources = fact.source_episodic_ids.len();
      let _ = writeln!(
        out,
        "- [{}] {} (sources: {} conversation{})",
        fact.category,
        fact.fact,
        sources,
        if sources == 1 { "" } else { "s" }
      );
    }
    let _ = writeln!(out);
  }

  // ── Episodic Memories ──
  if !episodic_results.is_empty() {
    let _ = writeln!(out, "## Episodic Memories");
  }

  let now = Utc::now();

  for (rank, (mem, score)) in episodic_results.iter().enumerate() {
    let rank = rank + 1; // 1-indexed

    // Header
    let key_moment = if mem.surprise >= 0.7 {
      ", key moment"
    } else {
      ""
    };
    let header = if mem.title.is_empty() {
      format!("Memory {rank}")
    } else {
      mem.title.clone()
    };
    let _ = writeln!(
      out,
      "### {header} [rank: {rank}, score: {score:.2}{key_moment}]"
    );

    // When
    let duration = now.signed_duration_since(mem.end_at);
    let relative = HumanTime::from(duration);
    let _ = writeln!(out, "**When:** {relative}");

    // Summary
    let _ = writeln!(out, "**Summary:** {}", mem.summary);

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
