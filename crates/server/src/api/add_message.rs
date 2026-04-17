use apalis::prelude::TaskSink;
use axum::{
  Json,
  extract::State,
  http::StatusCode,
  response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use plastmem_core::{append_batch_messages, append_message};
use plastmem_shared::{AppError, Message, MessageRole};
use plastmem_worker::EventSegmentationJob;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::utils::AppState;

// Input message type with timestamp optional
#[derive(Debug, Deserialize, ToSchema)]
pub struct InputMessage {
  pub role: MessageRole,
  pub content: String,
  #[serde(
    default,
    with = "chrono::serde::ts_milliseconds_option",
    skip_serializing_if = "Option::is_none"
  )]
  pub timestamp: Option<DateTime<Utc>>,
}

// Input message type with conversation ID
#[derive(Debug, Deserialize, ToSchema)]
pub struct InputConversationMessage {
  pub conversation_id: Uuid,
  pub message: InputMessage,
}

// Input batch messages type with conversation ID
#[derive(Debug, Deserialize, ToSchema)]
pub struct InputConversationMessages {
  pub conversation_id: Uuid,
  pub messages: Vec<InputMessage>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct IngestMessageResult {
  pub accepted: bool,
}

impl IngestMessageResult {
  const fn accepted() -> Self {
    Self { accepted: true }
  }
}

// Add a message of a conversation
#[utoipa::path(
  post,
  path = "/api/v0/add_message",
  request_body = InputConversationMessage,
  responses(
    (status = 200, description = "Message accepted", body = IngestMessageResult),
    (status = 400, description = "Invalid request - message content cannot be empty")
  )
)]
#[axum::debug_handler]
#[tracing::instrument(skip(state), fields(conversation_id = %payload.conversation_id))]
pub async fn add_message(
  State(state): State<AppState>,
  Json(payload): Json<InputConversationMessage>,
) -> Result<Response, AppError> {
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

  if let Some(claim) = append_message(payload.conversation_id, message, false, &state.db).await? {
    let mut job_storage = state.segmentation_job_storage.clone();
    job_storage
      .push(EventSegmentationJob::from_claim(claim))
      .await?;
  }

  Ok((StatusCode::OK, Json(IngestMessageResult::accepted())).into_response())
}

// Add a batch of messages to a conversation
#[utoipa::path(
  post,
  path = "/api/v0/import_batch_messages",
  request_body = InputConversationMessages,
  responses(
    (status = 200, description = "Batch import accepted", body = IngestMessageResult),
    (status = 400, description = "Invalid request - one or more messages are empty")
  )
)]
#[axum::debug_handler]
#[tracing::instrument(skip(state), fields(conversation_id = %payload.conversation_id, message_count = payload.messages.len()))]
pub async fn import_batch_messages(
  State(state): State<AppState>,
  Json(payload): Json<InputConversationMessages>,
) -> Result<Response, AppError> {
  if payload
    .messages
    .iter()
    .any(|message| message.content.is_empty())
  {
    return Err(AppError::new(anyhow::anyhow!(
      "Message content cannot be empty"
    )));
  }

  let messages = payload
    .messages
    .into_iter()
    .map(|message| Message {
      role: message.role,
      content: message.content,
      timestamp: message.timestamp.unwrap_or_else(Utc::now),
    })
    .collect::<Vec<_>>();

  if let Some(claim) = append_batch_messages(payload.conversation_id, &messages, &state.db).await? {
    let mut job_storage = state.segmentation_job_storage.clone();
    job_storage
      .push(EventSegmentationJob::from_claim(claim))
      .await?;
  }

  Ok((StatusCode::OK, Json(IngestMessageResult::accepted())).into_response())
}
