use crate::{
  core::EpisodicMemory, services::retrieval::retrieve_memories, state::AppState, utils::AppError,
};
use axum::extract::State;
use axum::{Json, http::StatusCode};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct RetrieveMemoryRaw {
  pub query: String,
  pub limit: Option<usize>,
}

#[derive(serde::Serialize)]
pub struct RetrieveMemoryRawResponse {
  pub memories: Vec<EpisodicMemory>,
}

#[axum::debug_handler]
pub async fn retrieve_memory_raw(
  State(state): State<AppState>,
  Json(payload): Json<RetrieveMemoryRaw>,
) -> Result<(StatusCode, Json<RetrieveMemoryRawResponse>), AppError> {
  let limit = payload.limit.unwrap_or(100);

  let memories_guard = state.memories.read().await;
  let all_memories: Vec<EpisodicMemory> = memories_guard.iter().cloned().collect();

  let results = retrieve_memories(&payload.query, &all_memories, limit);

  Ok((
    StatusCode::OK,
    Json(RetrieveMemoryRawResponse { memories: results }),
  ))
}
