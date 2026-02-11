use apalis::prelude::TaskSink;
use axum::{Json, extract::State, http::StatusCode};
use chrono::{DateTime, Utc};
use plast_mem_core::{Message, MessageQueue, MessageRole};
use plast_mem_shared::AppError;
use plast_mem_worker::EventSegmentationJob;
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
  let timestamp = payload.message.timestamp.unwrap_or_else(Utc::now);

  let message = Message {
    role: payload.message.role,
    content: payload.message.content,
    timestamp,
  };

  MessageQueue::push(payload.conversation_id, message, &state.db).await?;

  // Get messages from queue to pass to the job
  let queue = MessageQueue::get(payload.conversation_id, &state.db).await?;
  let mut job_storage = state.job_storage.clone();
  job_storage
    .push(EventSegmentationJob {
      conversation_id: payload.conversation_id,
      messages: queue.messages,
      check: true, // Let LLM decide whether to create memory
    })
    .await?;

  Ok(StatusCode::OK)
}
