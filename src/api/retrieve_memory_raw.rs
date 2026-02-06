use crate::api::retrieve_memory::RetrieveMemory;
use crate::utils::{AppError, AppState};
use axum::extract::State;
use axum::{Json, http::StatusCode};

#[axum::debug_handler]
pub async fn retrieve_memory_raw(
  State(_state): State<AppState>,
  Json(_payload): Json<RetrieveMemory>,
) -> Result<StatusCode, AppError> {
  Ok(StatusCode::NOT_IMPLEMENTED)
}
