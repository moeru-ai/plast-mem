use std::ops::Deref;

use apalis::prelude::Data;
use async_openai::types::{
  ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
  ChatCompletionRequestUserMessage,
};
use chrono::Utc;
use plast_mem_core::{EpisodicMemory, Message, MessageQueue, MessageRole};
use plast_mem_db_schema::episodic_memory;
use plast_mem_llm::{embed, generate_text};
use plast_mem_shared::AppError;
use sea_orm::{DatabaseConnection, EntityTrait};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::jobs::WorkerError;

/// Job for event segmentation with LLM check
/// - If `check` is true: LLM decides whether to create memory and returns summary if yes
/// - If `check` is false: LLM directly generates summary
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EventSegmentationJob {
  pub conversation_id: Uuid,
  pub messages: Vec<Message>,
  pub check: bool,
}

/// Calls LLM to either:
/// - If check=true: Decide whether to create memory, return Some(summary) if yes, None if no
/// - If check=false: Directly generate and return summary
async fn generate_summary_with_check(
  messages: &[Message],
  check: bool,
) -> Result<Option<String>, AppError> {
  let formatted_messages = messages
    .iter()
    .map(|m| {
      let role = match m.role {
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
      };
      format!("{}: {}", role, m.content)
    })
    .collect::<Vec<_>>()
    .join("\n");

  let system_prompt = if check {
    "You are an event segmentation analyzer. Analyze the conversation and decide if it contains significant content worth remembering as an episodic memory.\n\
     If the conversation is meaningful (contains important information, events, or context), reply with 'CREATE: ' followed by a concise summary.\n\
     If the conversation is trivial (greetings, small talk, or unimportant exchanges), reply with 'SKIP'.\n\
     Be selective - only mark as CREATE if there's substantive content."
  } else {
    "You are a professional summarizer. Provide a clear and concise summary of the following conversation."
  };

  let system = ChatCompletionRequestSystemMessage::from(system_prompt);
  let user = ChatCompletionRequestUserMessage::from(formatted_messages);

  let response = generate_text(vec![
    ChatCompletionRequestMessage::System(system),
    ChatCompletionRequestMessage::User(user),
  ])
  .await?;

  if check {
    let trimmed = response.trim();
    if trimmed.starts_with("CREATE:") {
      let summary = trimmed.strip_prefix("CREATE:").unwrap_or(trimmed).trim();
      if !summary.is_empty() {
        Ok(Some(summary.to_string()))
      } else {
        // If summary is empty after CREATE:, still create but use full response
        Ok(Some(trimmed.to_string()))
      }
    } else {
      // SKIP or any other response means don't create
      Ok(None)
    }
  } else {
    Ok(Some(response))
  }
}

pub async fn process_event_segmentation(
  job: EventSegmentationJob,
  db: Data<DatabaseConnection>,
) -> Result<(), WorkerError> {
  let db = db.deref();

  // If no messages, nothing to do
  if job.messages.is_empty() {
    return Ok(());
  }

  // Call LLM to get summary (with check logic)
  let Some(summary) = generate_summary_with_check(&job.messages, job.check).await? else {
    // LLM decided not to create memory (check=true case)
    // Still need to clear the messages from queue
    MessageQueue::drain(job.conversation_id, job.messages.len(), &db).await?;
    return Ok(());
  };

  // Generate embedding for the summary
  let embedding = embed(&summary).await?;

  let now = Utc::now();
  let start_at = job.messages.first().map(|m| m.timestamp).unwrap_or(now);
  let end_at = job.messages.last().map(|m| m.timestamp).unwrap_or(now);

  // Create EpisodicMemory directly without calling LLM again
  let episodic_memory = EpisodicMemory {
    id: Uuid::now_v7(),
    conversation_id: job.conversation_id,
    messages: job.messages.clone(),
    content: summary,
    embedding,
    start_at,
    end_at,
    created_at: now,
    last_reviewed_at: now,
  };

  // Insert into database
  let model = episodic_memory.to_model()?;
  let active_model: episodic_memory::ActiveModel = model.into();

  episodic_memory::Entity::insert(active_model)
    .exec(db)
    .await
    .map_err(AppError::from)?;

  // Clear the processed messages from MessageQueue
  MessageQueue::drain(job.conversation_id, job.messages.len(), &db).await?;

  Ok(())
}
