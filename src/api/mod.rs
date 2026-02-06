use axum::{
  Router,
  routing::{get, post},
};

use crate::utils::AppState;

mod add_message;
mod retrieve_memory;
mod retrieve_memory_raw;

pub fn app() -> Router<AppState> {
  Router::new()
    .route("/api/v0/add_message", post(add_message::add_message))
    .route(
      "/api/v0/retrieve_memory",
      get(retrieve_memory::retrieve_memory),
    )
    .route(
      "/api/v0/retrieve_memory_raw",
      get(retrieve_memory_raw::retrieve_memory_raw),
    )
}
