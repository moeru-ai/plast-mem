use axum::{Json, Router, routing::get};
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};
use utoipa_scalar::{Scalar, Servable};

use crate::utils::AppState;

mod add_message;
mod recent_memory;
mod retrieve_memory;
mod segmentation_state;

pub use add_message::{
  AddMessage, AddMessageMessage, AddMessageResult, ImportMessages, ImportMessagesResult,
};
pub use recent_memory::RecentMemory;
pub use retrieve_memory::{
  ContextPreRetrieve, EpisodicMemoryResult, RetrieveMemory, RetrieveMemoryRawResult,
  SemanticMemoryResult,
};
pub use segmentation_state::{SegmentationStateQuery, SegmentationStateStatus};

pub fn app() -> Router<AppState> {
  let router = OpenApiRouter::with_openapi(ApiDoc::openapi())
    .routes(routes!(add_message::add_message))
    .routes(routes!(add_message::messages_append))
    .routes(routes!(add_message::messages_import))
    .routes(routes!(recent_memory::recent_memory))
    .routes(routes!(recent_memory::recent_memory_raw))
    .routes(routes!(retrieve_memory::retrieve_memory))
    .routes(routes!(retrieve_memory::retrieve_memory_raw))
    .routes(routes!(retrieve_memory::context_pre_retrieve))
    .routes(routes!(segmentation_state::segmentation_state));

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
    ImportMessages,
    ImportMessagesResult,
    RecentMemory,
    SegmentationStateQuery,
    SegmentationStateStatus,
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
    ImportMessages,
    ImportMessagesResult,
    RecentMemory,
    SegmentationStateQuery,
    SegmentationStateStatus,
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
