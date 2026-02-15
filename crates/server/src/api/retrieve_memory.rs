use axum::{Json, extract::State};
use plastmem_core::{DetailLevel, EpisodicMemory, MessageQueue, format_tool_result};
use plastmem_shared::AppError;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::utils::AppState;

// --- Shared ---

const fn default_limit() -> u64 {
  5
}

#[derive(Deserialize, ToSchema)]
pub struct RetrieveMemory {
  /// Conversation ID to associate pending review with
  pub conversation_id: Uuid,
  /// Search query text
  pub query: String,
  /// Maximum memories to return (1-100)
  #[serde(default = "default_limit")]
  pub limit: u64,
  /// Detail level: "auto", "none", "low", "high"
  #[serde(default)]
  pub detail: DetailLevel,
  /// Optional scope: when set, only retrieve memories from this conversation.
  /// When omitted, searches all conversations (cross-conversation recall).
  pub scope: Option<Uuid>,
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
pub struct RetrieveMemoryRawResult {
  #[serde(flatten)]
  pub memory: EpisodicMemory,
  /// Final score (RRF score × retrievability)
  pub score: f64,
}

/// Retrieve memories in raw JSON format
#[utoipa::path(
  post,
  path = "/api/v0/retrieve_memory/raw",
  request_body = RetrieveMemory,
  responses(
    (status = 200, description = "List of memories with scores", body = Vec<RetrieveMemoryRawResult>),
    (status = 400, description = "Query cannot be empty")
  )
)]
#[axum::debug_handler]
pub async fn retrieve_memory_raw(
  State(state): State<AppState>,
  Json(payload): Json<RetrieveMemory>,
) -> Result<Json<Vec<RetrieveMemoryRawResult>>, AppError> {
  if payload.query.is_empty() {
    return Err(AppError::new(anyhow::anyhow!("Query cannot be empty")));
  }

  let results =
    EpisodicMemory::retrieve(&payload.query, payload.limit, payload.scope, &state.db)
      .await?;

  record_pending_review(&state, payload.conversation_id, &payload.query, &results).await?;

  let response = results
    .into_iter()
    .map(|(memory, score)| RetrieveMemoryRawResult { memory, score })
    .collect();

  Ok(Json(response))
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

  let results =
    EpisodicMemory::retrieve(&payload.query, payload.limit, payload.scope, &state.db)
      .await?;

  record_pending_review(&state, payload.conversation_id, &payload.query, &results).await?;

  Ok(format_tool_result(&results, &payload.detail))
}
