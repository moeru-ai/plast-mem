use axum::{
  Router,
  routing::post,
};

use crate::utils::AppState;

mod add_message;
mod retrieve_memory;

pub fn app() -> Router<AppState> {
  Router::new()
    .route("/api/v0/add_message", post(add_message::add_message))
    .route(
      "/api/v0/retrieve_memory",
      post(retrieve_memory::retrieve_memory),
    )
    .route(
      "/api/v0/retrieve_memory/raw",
      post(retrieve_memory::retrieve_memory_raw),
    )
}
