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

const fn default_episodic_limit() -> u64 { 5 }
const fn default_semantic_limit() -> u64 { 20 }

const fn sanitize_limit(value: u64) -> i64 {
  if value > 0 && value <= 1000 { value.cast_signed() } else { 100 }
}

#[derive(Debug, Deserialize, ToSchema)]
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

/// Fetch both memory types and record a pending review for episodic results.
async fn fetch_memory(
  state: &AppState,
  conversation_id: Uuid,
  query: &str,
  episodic_limit: u64,
  semantic_limit: u64,
) -> Result<(Vec<(SemanticMemory, f64)>, Vec<(EpisodicMemory, f64)>), AppError> {
  let (semantic, episodic) = tokio::try_join!(
    SemanticMemory::retrieve(query, sanitize_limit(semantic_limit), conversation_id, &state.db),
    EpisodicMemory::retrieve(query, episodic_limit, conversation_id, &state.db),
  )?;
  if !episodic.is_empty() {
    let memory_ids = episodic.iter().map(|(m, _)| m.id).collect();
    MessageQueue::add_pending_review(conversation_id, memory_ids, query.to_owned(), &state.db)
      .await?;
  }
  Ok((semantic, episodic))
}

// --- Pre-retrieval context endpoint (no pending review) ---

#[derive(Debug, Deserialize, ToSchema)]
pub struct ContextPreRetrieve {
  pub conversation_id: Uuid,
  pub query: String,
  #[serde(default = "default_semantic_limit")]
  pub semantic_limit: u64,
  #[serde(default)]
  pub detail: DetailLevel,
}

/// Retrieve semantic memories as markdown for pre-retrieval context injection.
/// Semantic-only (facts + behavioral guidelines); episodic retrieval is left to LLM tool calls.
/// Does NOT record a pending review (no FSRS update triggered).
#[utoipa::path(
  post,
  path = "/api/v0/context_pre_retrieve",
  request_body = ContextPreRetrieve,
  responses(
    (status = 200, description = "Markdown context for system prompt injection", body = String),
    (status = 400, description = "Query cannot be empty")
  )
)]
#[axum::debug_handler]
#[tracing::instrument(skip(state), fields(conversation_id = %payload.conversation_id))]
pub async fn context_pre_retrieve(
  State(state): State<AppState>,
  Json(payload): Json<ContextPreRetrieve>,
) -> Result<String, AppError> {
  if payload.query.is_empty() {
    return Err(AppError::new(anyhow::anyhow!("Query cannot be empty")));
  }
  let semantic = SemanticMemory::retrieve(
    &payload.query,
    sanitize_limit(payload.semantic_limit),
    payload.conversation_id,
    &state.db,
  )
  .await?;
  Ok(format_tool_result(&semantic, &[], &payload.detail))
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
#[tracing::instrument(skip(state), fields(conversation_id = %payload.conversation_id))]
pub async fn retrieve_memory_raw(
  State(state): State<AppState>,
  Json(payload): Json<RetrieveMemory>,
) -> Result<Json<RetrieveMemoryRawResult>, AppError> {
  if payload.query.is_empty() {
    return Err(AppError::new(anyhow::anyhow!("Query cannot be empty")));
  }
  let (semantic, episodic) = fetch_memory(
    &state,
    payload.conversation_id,
    &payload.query,
    payload.episodic_limit,
    payload.semantic_limit,
  )
  .await?;
  Ok(Json(RetrieveMemoryRawResult {
    semantic: semantic.into_iter().map(|(memory, score)| SemanticMemoryResult { memory, score }).collect(),
    episodic: episodic.into_iter().map(|(memory, score)| EpisodicMemoryResult { memory, score }).collect(),
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
#[tracing::instrument(skip(state), fields(conversation_id = %payload.conversation_id))]
pub async fn retrieve_memory(
  State(state): State<AppState>,
  Json(payload): Json<RetrieveMemory>,
) -> Result<String, AppError> {
  if payload.query.is_empty() {
    return Err(AppError::new(anyhow::anyhow!("Query cannot be empty")));
  }
  let (semantic, episodic) = fetch_memory(
    &state,
    payload.conversation_id,
    &payload.query,
    payload.episodic_limit,
    payload.semantic_limit,
  )
  .await?;
  Ok(format_tool_result(&semantic, &episodic, &payload.detail))
}
