use std::fmt::Write;

use serde::Deserialize;
use utoipa::ToSchema;

use super::EpisodicMemory;
use super::SemanticMemory;

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

#[must_use]
pub fn format_tool_result(
  semantic_results: &[(SemanticMemory, f64)],
  episodic_results: &[(EpisodicMemory, f64)],
  _detail: &DetailLevel,
) -> String {
  let mut out = String::new();

  // ── Episodic Memories ──
  if !episodic_results.is_empty() {
    let _ = writeln!(out, "## Episodic Memories");
  }
  for (mem, _score) in episodic_results {
    let _ = writeln!(out, "{}", mem.content);
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

#[cfg(test)]
mod tests {
  use chrono::Utc;
  use chrono::TimeZone;
  use sea_orm::prelude::PgVector;
  use uuid::Uuid;

  use super::*;
  use crate::memory::episodic::EpisodicMemory;
  use plastmem_shared::{Message, MessageRole};

  fn episodic_memory(content: &str) -> EpisodicMemory {
    EpisodicMemory {
      id: Uuid::now_v7(),
      conversation_id: Uuid::now_v7(),
      messages: vec![Message {
        role: MessageRole("Sam".to_owned()),
        content: "raw".to_owned(),
        timestamp: Utc.timestamp_opt(0, 0).single().expect("valid timestamp"),
      }],
      title: "Ignored title".to_owned(),
      content: content.to_owned(),
      classification: None,
      embedding: PgVector::from(vec![0.0; 1024]),
      stability: 1.0,
      difficulty: 1.0,
      surprise: 0.0,
      start_at: Utc.timestamp_opt(0, 0).single().expect("valid timestamp"),
      end_at: Utc.timestamp_opt(0, 0).single().expect("valid timestamp"),
      created_at: Utc.timestamp_opt(0, 0).single().expect("valid timestamp"),
      last_reviewed_at: Utc.timestamp_opt(0, 0).single().expect("valid timestamp"),
      consolidated_at: None,
    }
  }

  #[test]
  fn format_tool_result_outputs_only_episodic_content_blocks() {
    let episodic = vec![
      (episodic_memory("Spoken At: Jun 15, 2026 3 PM\nSam: hello"), 0.9),
      (episodic_memory("Spoken At: Jun 16, 2026 4 PM\nEvan: hi"), 0.8),
    ];

    let rendered = format_tool_result(&[], &episodic, &DetailLevel::Auto);

    assert_eq!(
      rendered,
      "## Episodic Memories\nSpoken At: Jun 15, 2026 3 PM\nSam: hello\n\nSpoken At: Jun 16, 2026 4 PM\nEvan: hi"
    );
    assert!(!rendered.contains("### "));
    assert!(!rendered.contains("**Conversation Time:**"));
    assert!(!rendered.contains("**Details:**"));
    assert!(!rendered.contains("**Time Evidence:**"));
  }
}
