use apalis::prelude::TaskSink;
use axum::{Json, extract::State, http::StatusCode};
use chrono::{DateTime, Utc};
use plast_mem_shared::AppError;
use serde::Deserialize;
use uuid::Uuid;

use crate::{
  core::{Message, MessageQueue, MessageRole},
  utils::AppState,
};
use plast_mem_worker::{MessageQueueSegmentJob, WorkerJob};

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
  let mut job_storage = state.job_storage.clone();
  job_storage
    .push(WorkerJob::Segment(MessageQueueSegmentJob {
      conversation_id: payload.conversation_id,
    }))
    .await?;

  Ok(StatusCode::OK)
}
