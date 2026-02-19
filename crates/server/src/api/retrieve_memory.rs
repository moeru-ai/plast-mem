use axum::{Json, extract::State};
use plastmem_core::{
  DetailLevel, EpisodicMemory, MessageQueue, SemanticMemory, format_tool_result,
};
use plastmem_shared::AppError;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::utils::AppState;

// --- Shared ---

const fn default_episodic_limit() -> u64 {
  5
}

const fn default_semantic_limit() -> u64 {
  20
}

#[derive(Deserialize, ToSchema)]
pub struct RetrieveMemory {
  /// Conversation ID to filter memories by and associate pending review with
  pub conversation_id: Uuid,
  /// Search query text
  pub query: String,
  /// Maximum episodic memories to return (1-100)
  #[serde(default = "default_episodic_limit")]
  pub episodic_limit: u64,
  /// Maximum semantic memories to return
  #[serde(default = "default_semantic_limit")]
  pub semantic_limit: u64,
  /// Detail level: "auto", "none", "low", "high"
  #[serde(default)]
  pub detail: DetailLevel,
}

/// Record retrieved memory IDs as pending review in the message queue.
async fn record_pending_review(
  state: &AppState,
  conversation_id: Uuid,
  query: &str,
  results: &[(EpisodicMemory, f64)],
) -> Result<(), AppError> {
  if !results.is_empty() {
    let memory_ids = results.iter().map(|(m, _)| m.id).collect();
    MessageQueue::add_pending_review(conversation_id, memory_ids, query.to_owned(), &state.db)
      .await?;
  }
  Ok(())
}

// --- Raw JSON endpoint ---

#[derive(Serialize, ToSchema)]
pub struct SemanticMemoryResult {
  #[serde(flatten)]
  pub memory: SemanticMemory,
  /// RRF score
  pub score: f64,
}

#[derive(Serialize, ToSchema)]
pub struct RetrieveMemoryRawResult {
  /// Semantic memories (known facts + behavioral guidelines)
  pub semantic: Vec<SemanticMemoryResult>,
  /// Episodic memories with scores
  pub episodic: Vec<EpisodicMemoryResult>,
}

#[derive(Serialize, ToSchema)]
pub struct EpisodicMemoryResult {
  #[serde(flatten)]
  pub memory: EpisodicMemory,
  /// Final score (RRF score × FSRS retrievability)
  pub score: f64,
}

/// Retrieve memories in raw JSON format
#[utoipa::path(
  post,
  path = "/api/v0/retrieve_memory/raw",
  request_body = RetrieveMemory,
  responses(
    (status = 200, description = "Semantic facts and episodic memories", body = RetrieveMemoryRawResult),
    (status = 400, description = "Query cannot be empty")
  )
)]
#[axum::debug_handler]
pub async fn retrieve_memory_raw(
  State(state): State<AppState>,
  Json(payload): Json<RetrieveMemory>,
) -> Result<Json<RetrieveMemoryRawResult>, AppError> {
  if payload.query.is_empty() {
    return Err(AppError::new(anyhow::anyhow!("Query cannot be empty")));
  }

  let (semantic_results, episodic_results) = tokio::try_join!(
    SemanticMemory::retrieve(
      &payload.query,
      payload.semantic_limit,
      payload.conversation_id,
      &state.db
    ),
    EpisodicMemory::retrieve(
      &payload.query,
      payload.episodic_limit,
      payload.conversation_id,
      &state.db
    ),
  )?;

  record_pending_review(
    &state,
    payload.conversation_id,
    &payload.query,
    &episodic_results,
  )
  .await?;

  Ok(Json(RetrieveMemoryRawResult {
    semantic: semantic_results
      .into_iter()
      .map(|(memory, score)| SemanticMemoryResult { memory, score })
      .collect(),
    episodic: episodic_results
      .into_iter()
      .map(|(memory, score)| EpisodicMemoryResult { memory, score })
      .collect(),
  }))
}

// --- Tool result (markdown) endpoint ---

/// Retrieve memories formatted as markdown for LLM consumption
#[utoipa::path(
  post,
  path = "/api/v0/retrieve_memory",
  request_body = RetrieveMemory,
  responses(
    (status = 200, description = "Markdown formatted memory results", body = String),
    (status = 400, description = "Query cannot be empty")
  )
)]
#[axum::debug_handler]
pub async fn retrieve_memory(
  State(state): State<AppState>,
  Json(payload): Json<RetrieveMemory>,
) -> Result<String, AppError> {
  if payload.query.is_empty() {
    return Err(AppError::new(anyhow::anyhow!("Query cannot be empty")));
  }

  let (semantic_results, episodic_results) = tokio::try_join!(
    SemanticMemory::retrieve(
      &payload.query,
      payload.semantic_limit,
      payload.conversation_id,
      &state.db
    ),
    EpisodicMemory::retrieve(
      &payload.query,
      payload.episodic_limit,
      payload.conversation_id,
      &state.db
    ),
  )?;

  record_pending_review(
    &state,
    payload.conversation_id,
    &payload.query,
    &episodic_results,
  )
  .await?;

  Ok(format_tool_result(
    &semantic_results,
    &episodic_results,
    &payload.detail,
  ))
}
