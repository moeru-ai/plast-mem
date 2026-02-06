use crate::utils::{AppError, AppState};
use axum::extract::State;
use axum::{Json, http::StatusCode};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct RetrieveMemory {
  pub query: String,
  pub limit: Option<usize>,
}

#[axum::debug_handler]
pub async fn retrieve_memory(
  State(_state): State<AppState>,
  Json(_payload): Json<RetrieveMemory>,
) -> Result<StatusCode, AppError> {
  Ok(StatusCode::NOT_IMPLEMENTED)
}
