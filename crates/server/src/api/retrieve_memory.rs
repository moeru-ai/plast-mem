use apalis::prelude::TaskSink;
use axum::{Json, extract::State};
use chrono::Utc;
use plastmem_core::{DetailLevel, EpisodicMemory, format_tool_result};
use plastmem_shared::AppError;
use plastmem_worker::MemoryReviewJob;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::utils::AppState;

// --- Shared ---

const fn default_limit() -> usize {
  5
}

#[derive(Deserialize, ToSchema)]
pub struct RetrieveMemory {
  /// Search query text
  pub query: String,
  /// Maximum memories to return (1-100)
  #[serde(default = "default_limit")]
  pub limit: usize,
  /// Detail level: "auto", "none", "low", "high"
  #[serde(default)]
  pub detail: DetailLevel,
}

async fn enqueue_review_job(
  state: &AppState,
  results: &[(EpisodicMemory, f64)],
) -> Result<(), AppError> {
  if !results.is_empty() {
    let memory_ids = results.iter().map(|(m, _)| m.id).collect();
    let reviewed_at = Utc::now();
    let mut review_storage = state.review_job_storage.clone();
    review_storage
      .push(MemoryReviewJob {
        memory_ids,
        reviewed_at,
      })
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

  let results = EpisodicMemory::retrieve(&payload.query, payload.limit as u64, &state.db).await?;

  enqueue_review_job(&state, &results).await?;

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

  let results = EpisodicMemory::retrieve(&payload.query, payload.limit as u64, &state.db).await?;

  enqueue_review_job(&state, &results).await?;

  Ok(format_tool_result(&results, &payload.detail))
}
