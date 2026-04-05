use apalis::prelude::TaskSink;
use axum::{
  Json,
  extract::State,
  http::StatusCode,
  response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use plastmem_core::{
  ADD_BACKPRESSURE_LIMIT, SEGMENTATION_IN_PROGRESS_TTL_MINUTES, append_messages,
  clear_stale_in_progress, get_processing_status,
};
use plastmem_shared::{AppError, Message, MessageRole};
use plastmem_worker::EventSegmentationJob;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::utils::AppState;

#[derive(Debug, Deserialize, ToSchema)]
pub struct AddMessage {
  pub conversation_id: Uuid,
  pub message: AddMessageMessage,
}

#[derive(Debug, Deserialize, ToSchema, Clone)]
pub struct AddMessageMessage {
  pub role: MessageRole,
  pub content: String,
  #[serde(
    default,
    with = "chrono::serde::ts_milliseconds_option",
    skip_serializing_if = "Option::is_none"
  )]
  pub timestamp: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AddMessageResult {
  pub accepted: bool,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub reason: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ImportMessages {
  pub conversation_id: Uuid,
  pub messages: Vec<AddMessageMessage>,
  #[serde(default)]
  pub eof: bool,
  #[serde(default)]
  pub import_id: Option<Uuid>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ImportMessagesResult {
  pub accepted: bool,
  pub inserted_count: i64,
  pub eof: bool,
}

impl AddMessageResult {
  const fn accepted() -> Self {
    Self {
      accepted: true,
      reason: None,
    }
  }

  fn backpressure() -> Self {
    Self {
      accepted: false,
      reason: Some("backpressure".to_owned()),
    }
  }
}

/// Legacy append alias that now routes into the v2 ingestion path.
#[utoipa::path(
  post,
  path = "/api/v0/add_message",
  request_body = AddMessage,
  responses(
    (status = 200, description = "Message accepted", body = AddMessageResult),
    (status = 429, description = "Backpressured - message not accepted", body = AddMessageResult),
    (status = 400, description = "Invalid request - message content cannot be empty")
  )
)]
#[axum::debug_handler]
#[tracing::instrument(skip(state), fields(conversation_id = %payload.conversation_id))]
pub async fn add_message(
  State(state): State<AppState>,
  Json(payload): Json<AddMessage>,
) -> Result<Response, AppError> {
  append_single_message(state, payload).await
}

#[utoipa::path(
  post,
  path = "/api/v1/messages:append",
  request_body = AddMessage,
  responses(
    (status = 200, description = "Message accepted", body = AddMessageResult),
    (status = 429, description = "Backpressured - message not accepted", body = AddMessageResult),
    (status = 400, description = "Invalid request - message content cannot be empty")
  )
)]
#[axum::debug_handler]
#[tracing::instrument(skip(state), fields(conversation_id = %payload.conversation_id))]
pub async fn messages_append(
  State(state): State<AppState>,
  Json(payload): Json<AddMessage>,
) -> Result<Response, AppError> {
  append_single_message(state, payload).await
}

#[utoipa::path(
  post,
  path = "/api/v1/messages:import",
  request_body = ImportMessages,
  responses(
    (status = 200, description = "Messages imported", body = ImportMessagesResult),
    (status = 400, description = "Invalid request")
  )
)]
#[axum::debug_handler]
#[tracing::instrument(skip(state), fields(conversation_id = %payload.conversation_id))]
pub async fn messages_import(
  State(state): State<AppState>,
  Json(payload): Json<ImportMessages>,
) -> Result<Json<ImportMessagesResult>, AppError> {
  if payload.messages.iter().any(|message| message.content.is_empty()) {
    return Err(AppError::new(anyhow::anyhow!(
      "Message content cannot be empty"
    )));
  }

  let import_id = payload.import_id.unwrap_or_else(Uuid::now_v7);
  let messages = payload
    .messages
    .into_iter()
    .map(normalize_message)
    .collect::<Vec<_>>();

  let result = append_messages(
    payload.conversation_id,
    &messages,
    payload.eof,
    Some("batch_import"),
    Some(import_id),
    &state.db,
  )
  .await?;

  if result.job_enqueued {
    enqueue_segmentation_job(payload.conversation_id, &state).await?;
  }

  Ok(Json(ImportMessagesResult {
    accepted: true,
    inserted_count: result.inserted_count,
    eof: payload.eof,
  }))
}

async fn append_single_message(state: AppState, payload: AddMessage) -> Result<Response, AppError> {
  if payload.message.content.is_empty() {
    return Err(AppError::new(anyhow::anyhow!(
      "Message content cannot be empty"
    )));
  }

  if is_backpressured(payload.conversation_id, &state.db).await? {
    return Ok(
      (
        StatusCode::TOO_MANY_REQUESTS,
        Json(AddMessageResult::backpressure()),
      )
        .into_response(),
    );
  }

  let message = normalize_message(payload.message);
  let result = append_messages(
    payload.conversation_id,
    &[message],
    false,
    Some("streaming"),
    None,
    &state.db,
  )
  .await?;

  if result.job_enqueued {
    enqueue_segmentation_job(payload.conversation_id, &state).await?;
  }

  Ok((StatusCode::OK, Json(AddMessageResult::accepted())).into_response())
}

fn normalize_message(payload: AddMessageMessage) -> Message {
  Message {
    role: payload.role,
    content: payload.content,
    timestamp: payload.timestamp.unwrap_or_else(Utc::now),
  }
}

async fn enqueue_segmentation_job(
  conversation_id: Uuid,
  state: &AppState,
) -> Result<(), AppError> {
  let mut job_storage = state.segmentation_job_storage.clone();
  job_storage.push(EventSegmentationJob { conversation_id }).await?;
  Ok(())
}

async fn is_backpressured(
  conversation_id: Uuid,
  db: &sea_orm::DatabaseConnection,
) -> Result<bool, AppError> {
  let mut status = get_processing_status(conversation_id, db).await?;
  if status.fence_active
    && clear_stale_in_progress(conversation_id, SEGMENTATION_IN_PROGRESS_TTL_MINUTES, db).await?
  {
    status = get_processing_status(conversation_id, db).await?;
  }

  Ok(status.fence_active && status.messages_pending >= ADD_BACKPRESSURE_LIMIT)
}
