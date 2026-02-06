use axum::{Json, http::StatusCode};
use serde::Deserialize;
use crate::{
  core::{EpisodicMemory, format_messages_with_date},
  services::retrieval::retrieve_memories,
  state::AppState,
  utils::AppError,
};
use axum::extract::State;

#[derive(Deserialize)]
pub struct RetrieveMemory {
  pub query: String,
  pub limit: Option<usize>,
}

#[derive(serde::Serialize)]
pub struct RetrieveMemoryResponse {
  pub text: String,
}

#[axum::debug_handler]
pub async fn retrieve_memory(
  State(state): State<AppState>,
  Json(payload): Json<RetrieveMemory>,
) -> Result<(StatusCode, Json<RetrieveMemoryResponse>), AppError> {
  let limit = payload.limit.unwrap_or(100);

  let memories_guard = state.memories.read().await;
  let all_memories: Vec<EpisodicMemory> = memories_guard.iter().cloned().collect();

  let results = retrieve_memories(&payload.query, &all_memories, limit);
  let mut parts = Vec::new();
  for mem in results {
    parts.push(format_messages_with_date(&mem.messages));
  }

  Ok((
    StatusCode::OK,
    Json(RetrieveMemoryResponse {
      text: parts.join("\n\n"),
    }),
  ))
}
