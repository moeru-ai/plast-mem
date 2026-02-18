use plastmem_core::process_extraction;
use plastmem_shared::{AppError, Message};
use sea_orm::DatabaseConnection;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ──────────────────────────────────────────────────
// Job definition
// ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticExtractionJob {
  pub episode_id: Uuid,
  pub conversation_id: Uuid,
  pub summary: String,
  pub messages: Vec<Message>,
  pub surprise: f32,
}

// ──────────────────────────────────────────────────
// Job processing
// ──────────────────────────────────────────────────

pub async fn process_semantic_extraction(
  job: SemanticExtractionJob,
  db: apalis::prelude::Data<DatabaseConnection>,
) -> Result<(), AppError> {
  let db = &*db;

  tracing::info!(
    episode_id = %job.episode_id,
    conversation_id = %job.conversation_id,
    surprise = job.surprise,
    "Processing semantic extraction"
  );

  process_extraction(
    job.episode_id,
    &job.summary,
    &job.messages,
    job.surprise,
    db,
  )
  .await?;

  Ok(())
}
