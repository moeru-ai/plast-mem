use axum::{Json, extract::State};
use plast_mem_core::EpisodicMemory;
use plast_mem_shared::AppError;
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

  let response = results
    .into_iter()
    .map(|(memory, score)| RetrieveMemoryResult { memory, score })
    .collect();

  Ok(Json(response))
}
