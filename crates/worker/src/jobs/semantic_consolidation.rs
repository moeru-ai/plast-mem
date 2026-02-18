use plastmem_core::{
  CONSOLIDATION_EPISODE_THRESHOLD, EpisodicMemory, process_consolidation,
};
use plastmem_shared::AppError;
use sea_orm::DatabaseConnection;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ──────────────────────────────────────────────────
// Job definition
// ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticConsolidationJob {
  pub conversation_id: Uuid,
  /// If true, consolidate even if below the episode threshold (e.g., flashbulb trigger).
  pub force: bool,
}

// ──────────────────────────────────────────────────
// Job processing
// ──────────────────────────────────────────────────

pub async fn process_semantic_consolidation(
  job: SemanticConsolidationJob,
  db: apalis::prelude::Data<DatabaseConnection>,
) -> Result<(), AppError> {
  let db = &*db;

  // Fetch unconsolidated episodes for this specific conversation
  let episodes =
    EpisodicMemory::fetch_unconsolidated_for_conversation(job.conversation_id, db).await?;

  if episodes.is_empty() {
    tracing::debug!(
      conversation_id = %job.conversation_id,
      "No unconsolidated episodes, skipping consolidation"
    );
    return Ok(());
  }

  // Check threshold (unless force-triggered)
  if !job.force && (episodes.len() as u64) < CONSOLIDATION_EPISODE_THRESHOLD {
    tracing::debug!(
      conversation_id = %job.conversation_id,
      episodes = episodes.len(),
      threshold = CONSOLIDATION_EPISODE_THRESHOLD,
      "Below consolidation threshold, skipping"
    );
    return Ok(());
  }

  tracing::info!(
    conversation_id = %job.conversation_id,
    episodes = episodes.len(),
    force = job.force,
    "Processing semantic consolidation"
  );

  process_consolidation(&episodes, db).await?;

  Ok(())
}
