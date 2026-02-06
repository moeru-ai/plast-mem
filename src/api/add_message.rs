use axum::{Json, extract::State, http::StatusCode};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
  core::{
    EpisodicMemory, Message, MessageQueue, MessageRole, SegmentDecision, SegmenterFn,
    llm_segmenter, rule_segmenter,
  },
  state::AppState,
  utils::AppError,
};

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

  let mut message_queue = MessageQueue::new(payload.conversation_id, &state.db).await?;
  message_queue.push(message, &state.db).await?;

  Ok(StatusCode::OK)
}
