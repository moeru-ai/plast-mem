use apalis::prelude::TaskSink;
use axum::{
  Json,
  extract::State,
  http::StatusCode,
  response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use plastmem_core::{ADD_BACKPRESSURE_LIMIT, FENCE_TTL_MINUTES, MessageQueue};
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

#[derive(Debug, Deserialize, ToSchema)]
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

impl AddMessageResult {
  fn accepted() -> Self {
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

/// Add a message to a conversation
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

  let timestamp = payload.message.timestamp.unwrap_or_else(Utc::now);

  let message = Message {
    role: payload.message.role,
    content: payload.message.content,
    timestamp,
  };

  if let Some(check) = MessageQueue::push(payload.conversation_id, message, &state.db).await? {
    let mut job_storage = state.job_storage.clone();
    job_storage
      .push(EventSegmentationJob {
        conversation_id: payload.conversation_id,
        fence_count: check.fence_count,
        force_process: check.force_process,
        keep_tail_segment: true,
      })
      .await?;
  }

  Ok((StatusCode::OK, Json(AddMessageResult::accepted())).into_response())
}

async fn is_backpressured(
  conversation_id: Uuid,
  db: &sea_orm::DatabaseConnection,
) -> Result<bool, AppError> {
  let mut status = MessageQueue::get_processing_status(conversation_id, db).await?;

  if status.fence_active
    && MessageQueue::clear_stale_fence(conversation_id, FENCE_TTL_MINUTES, db).await?
  {
    status = MessageQueue::get_processing_status(conversation_id, db).await?;
  }

  Ok(status.fence_active && status.messages_pending >= ADD_BACKPRESSURE_LIMIT)
}
