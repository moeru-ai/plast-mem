use apalis::prelude::TaskSink;
use axum::{Json, extract::State};
use chrono::Utc;
use plast_mem_core::EpisodicMemory;
use plast_mem_shared::AppError;
use plast_mem_worker::MemoryReviewJob;
use serde::{Deserialize, Serialize};

use crate::utils::AppState;

#[derive(Deserialize)]
pub struct RetrieveMemory {
  pub query: String,
}

#[derive(Serialize)]
pub struct RetrieveMemoryResult {
  #[serde(flatten)]
  pub memory: EpisodicMemory,
  pub score: f64,
}

#[axum::debug_handler]
pub async fn retrieve_memory(
  State(state): State<AppState>,
  Json(payload): Json<RetrieveMemory>,
) -> Result<Json<Vec<RetrieveMemoryResult>>, AppError> {
  let results = EpisodicMemory::retrieve(&payload.query, 5, &state.db).await?;

  // Push review job to update FSRS parameters asynchronously
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

  let response = results
    .into_iter()
    .map(|(memory, score)| RetrieveMemoryResult { memory, score })
    .collect();

  Ok(Json(response))
}
