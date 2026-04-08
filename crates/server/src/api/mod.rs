use axum::{Json, Router, routing::get};
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};
use utoipa_scalar::{Scalar, Servable};

use crate::utils::AppState;

mod add_message;
#[cfg(debug_assertions)]
mod benchmark;
mod recent_memory;
mod retrieve_memory;

pub use add_message::{AddMessage, AddMessageMessage, AddMessageResult};
#[cfg(debug_assertions)]
pub use benchmark::{BenchmarkFlush, BenchmarkFlushResult, BenchmarkJobStatus};
pub use recent_memory::RecentMemory;
pub use retrieve_memory::{
  ContextPreRetrieve, EpisodicMemoryResult, RetrieveMemory, RetrieveMemoryRawResult,
  SemanticMemoryResult,
};

pub fn app() -> Router<AppState> {
  let router = OpenApiRouter::with_openapi(ApiDoc::openapi())
    .routes(routes!(add_message::add_message))
    .routes(routes!(recent_memory::recent_memory))
    .routes(routes!(recent_memory::recent_memory_raw))
    .routes(routes!(retrieve_memory::retrieve_memory))
    .routes(routes!(retrieve_memory::retrieve_memory_raw))
    .routes(routes!(retrieve_memory::context_pre_retrieve));

  #[cfg(debug_assertions)]
  let router = router
    .routes(routes!(benchmark::benchmark_flush))
    .routes(routes!(benchmark::benchmark_job_status));

  let (router, openapi) = router.split_for_parts();

  let openapi_json = openapi.clone();

  router
    .route(
      "/openapi.json",
      get(move || async move { Json(openapi_json) }),
    )
    .merge(Scalar::with_url("/openapi/", openapi))
}

#[cfg(debug_assertions)]
#[derive(OpenApi)]
#[openapi(
  info(title = "Plast Mem"),
  components(schemas(
    AddMessage,
    AddMessageMessage,
    AddMessageResult,
    BenchmarkFlush,
    BenchmarkFlushResult,
    BenchmarkJobStatus,
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

#[cfg(not(debug_assertions))]
#[derive(OpenApi)]
#[openapi(
  info(title = "Plast Mem"),
  components(schemas(
    AddMessage,
    AddMessageMessage,
    AddMessageResult,
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
