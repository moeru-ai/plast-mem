use apalis::prelude::TaskSink;
use axum::{Json, extract::State, http::StatusCode};
use chrono::{DateTime, Utc};
use plastmem_core::{Message, MessageQueue, MessageRole};
use plastmem_shared::AppError;
use plastmem_worker::EventSegmentationJob;
use serde::Deserialize;
use uuid::Uuid;

use crate::utils::AppState;

#[derive(Deserialize)]
pub struct AddMessage {
  pub conversation_id: Uuid,
  pub message: AddMessageMessage,
}

#[derive(Deserialize)]
pub struct AddMessageMessage {
  pub role: MessageRole,
  pub content: String,
  #[serde(
    with = "chrono::serde::ts_milliseconds_option",
    skip_serializing_if = "Option::is_none"
  )]
  pub timestamp: Option<DateTime<Utc>>,
}

#[axum::debug_handler]
pub async fn add_message(
  State(state): State<AppState>,
  Json(payload): Json<AddMessage>,
) -> Result<StatusCode, AppError> {
  if payload.message.content.is_empty() {
    return Err(AppError::new(anyhow::anyhow!("Message content cannot be empty")));
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
        check: check.check,
        boundary_hint: check.boundary_hint,
      })
      .await?;
  }

  Ok(StatusCode::OK)
}
