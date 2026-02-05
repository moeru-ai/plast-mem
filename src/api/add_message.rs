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
  pub conversation_id: Option<Uuid>,
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
) -> Result<(StatusCode, Json<Message>), AppError> {
  let conversation_id = payload.conversation_id.unwrap_or_else(Uuid::now_v7);
  let timestamp = payload.message.timestamp.unwrap_or_else(Utc::now);

  let message = Message {
    role: payload.message.role,
    content: payload.message.content,
    timestamp,
  };

  let mut message_queues = state.message_queues.write().await;
  let message_queue = match message_queues
    .iter_mut()
    .find(|q| q.conversation_id == conversation_id)
  {
    Some(message_queue) => message_queue,
    None => {
      message_queues.push(MessageQueue::new(conversation_id));
      message_queues
        .last_mut()
        .expect("message_queue just inserted")
    }
  };

  let llm_segmenter_fn: &SegmenterFn = &llm_segmenter;
  if should_split(message_queue, &message, llm_segmenter_fn) {
    if !message_queue.messages.is_empty() {
      let messages = std::mem::take(&mut message_queue.messages);
      let memory = EpisodicMemory::new(conversation_id, messages);
      let mut memories = state.memories.write().await;
      memories.push(memory);
    }
  }

  message_queue.messages.push(message.clone());

  Ok((StatusCode::OK, Json(message)))
}

fn should_split(
  message_queue: &MessageQueue,
  incoming: &Message,
  llm_segmenter_fn: &SegmenterFn,
) -> bool {
  let recent: Vec<Message> = message_queue
    .messages
    .iter()
    .rev()
    .take(10)
    .cloned()
    .collect();
  let mut recent = recent;
  recent.reverse();

  match rule_segmenter(&recent, incoming) {
    SegmentDecision::Split => true,
    SegmentDecision::NoSplit => false,
    SegmentDecision::CallLlm => llm_segmenter_fn(&recent, incoming),
  }
}
