use axum::{Json, Router, routing::get};
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};
use utoipa_scalar::{Scalar, Servable};

use crate::utils::AppState;

mod add_message;
mod recent_memory;
mod retrieve_memory;

pub use add_message::{AddMessage, AddMessageMessage};
pub use recent_memory::RecentMemory;
pub use retrieve_memory::{
  ContextPreRetrieve, EpisodicMemoryResult, RetrieveMemory, RetrieveMemoryRawResult,
  SemanticMemoryResult,
};

pub fn app() -> Router<AppState> {
  let (router, openapi) = OpenApiRouter::with_openapi(ApiDoc::openapi())
    .routes(routes!(add_message::add_message))
    .routes(routes!(recent_memory::recent_memory))
    .routes(routes!(recent_memory::recent_memory_raw))
    .routes(routes!(retrieve_memory::retrieve_memory))
    .routes(routes!(retrieve_memory::retrieve_memory_raw))
    .routes(routes!(retrieve_memory::context_pre_retrieve))
    .split_for_parts();

  let openapi_json = openapi.clone();

  router
    .route(
      "/openapi.json",
      get(move || async move { Json(openapi_json) }),
    )
    .merge(Scalar::with_url("/openapi/", openapi))
}

#[derive(OpenApi)]
#[openapi(
  info(title = "Plast Mem"),
  components(schemas(
    AddMessage,
    AddMessageMessage,
    RecentMemory,
    RetrieveMemory,
    ContextPreRetrieve,
    RetrieveMemoryRawResult,
    EpisodicMemoryResult,
    SemanticMemoryResult,
    plastmem_core::EpisodicMemory,
    plastmem_core::SemanticMemory,
    plastmem_core::DetailLevel,
    plastmem_shared::Message,
    plastmem_shared::MessageRole,
  ))
)]
pub struct ApiDoc;
