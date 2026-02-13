use apalis::prelude::TaskSink;
use axum::{Json, extract::State};
use chrono::Utc;
use plastmem_core::{DetailLevel, EpisodicMemory, format_tool_result};
use plastmem_shared::AppError;
use plastmem_worker::MemoryReviewJob;
use serde::{Deserialize, Serialize};

use crate::utils::AppState;

// --- Shared ---

fn default_limit() -> usize {
  5
}

#[derive(Deserialize)]
pub struct RetrieveMemory {
  pub query: String,
  #[serde(default = "default_limit")]
  pub limit: usize,
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

#[derive(Serialize)]
pub struct RetrieveMemoryRawResult {
  #[serde(flatten)]
  pub memory: EpisodicMemory,
  pub score: f64,
}

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

  Ok(format_tool_result(&results, payload.detail))
}
