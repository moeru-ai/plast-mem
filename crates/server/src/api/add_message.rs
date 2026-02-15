use apalis::prelude::TaskSink;
use axum::{Json, extract::State, http::StatusCode};
use chrono::{DateTime, Utc};
use plastmem_core::{Message, MessageQueue, MessageRole};
use plastmem_shared::AppError;
use plastmem_worker::EventSegmentationJob;
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::utils::AppState;

#[derive(Deserialize, ToSchema)]
pub struct AddMessage {
  pub conversation_id: Uuid,
  pub message: AddMessageMessage,
}

#[derive(Deserialize, ToSchema)]
pub struct AddMessageMessage {
  pub role: MessageRole,
  pub content: String,
  #[serde(
    with = "chrono::serde::ts_milliseconds_option",
    skip_serializing_if = "Option::is_none"
  )]
  pub timestamp: Option<DateTime<Utc>>,
}

/// Add a message to a conversation
#[utoipa::path(
  post,
  path = "/api/v0/add_message",
  request_body = AddMessage,
  responses(
    (status = 200, description = "Message added successfully"),
    (status = 400, description = "Invalid request - message content cannot be empty")
  )
)]
#[axum::debug_handler]
pub async fn add_message(
  State(state): State<AppState>,
  Json(payload): Json<AddMessage>,
) -> Result<StatusCode, AppError> {
  if payload.message.content.is_empty() {
    return Err(AppError::new(anyhow::anyhow!(
      "Message content cannot be empty"
    )));
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
        messages: check.messages,
        action: check.action,
      })
      .await?;
  }

  Ok(StatusCode::OK)
}
