use axum::{
  Json, Router,
  routing::{get, post},
};
use utoipa::OpenApi;
use utoipa_scalar::{Scalar, Servable};

use crate::utils::AppState;

mod add_message;
mod retrieve_memory;

pub use add_message::{AddMessage, AddMessageMessage};
pub use retrieve_memory::{RetrieveMemory, RetrieveMemoryRawResult};

#[derive(OpenApi)]
#[openapi(
  info(
    title = "Plast Mem API",
    version = "0.0.1",
    description = "Experimental LLM memory layer for cyber waifu"
  ),
  paths(
    add_message::add_message,
    retrieve_memory::retrieve_memory,
    retrieve_memory::retrieve_memory_raw
  ),
  components(schemas(
    AddMessage,
    AddMessageMessage,
    RetrieveMemory,
    RetrieveMemoryRawResult,
    plastmem_core::EpisodicMemory,
    plastmem_core::DetailLevel,
    plastmem_shared::Message,
    plastmem_shared::MessageRole,
  ))
)]
pub struct ApiDoc;

async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
  Json(ApiDoc::openapi())
}

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
    .route("/openapi.json", get(openapi_json))
    .merge(Scalar::with_url("/openapi/", ApiDoc::openapi()))
}
