use std::ops::Deref;

use apalis::prelude::Data;
use chrono::Utc;
use fsrs::{DEFAULT_PARAMETERS, FSRS};
use plast_mem_core::{EpisodicMemory, Message, MessageQueue};
use plast_mem_db_schema::episodic_memory;
use plast_mem_llm::{embed, summarize_messages_with_check};
use plast_mem_shared::AppError;
use sea_orm::{DatabaseConnection, EntityTrait};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::jobs::WorkerError;

/// Target retention probability (90%).
const DESIRED_RETENTION: f32 = 0.9;

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
  summarize_messages_with_check(messages, check).await
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

  // Verify the queue hasn't been modified since job creation
  let message_queue = MessageQueue::get(job.conversation_id, db).await?;
  if job.messages.first().map(|m| (&m.content, m.timestamp))
    != message_queue
      .messages
      .first()
      .map(|m| (&m.content, m.timestamp))
  {
    // Queue has been modified, skip this stale job
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

  let id = Uuid::now_v7();
  let now = Utc::now();
  let start_at = job.messages.first().map(|m| m.timestamp).unwrap_or(now);
  let end_at = job.messages.last().map(|m| m.timestamp).unwrap_or(now);
  let messages_len = job.messages.len();

  // Initialize FSRS state for new memory
  let fsrs = FSRS::new(Some(&DEFAULT_PARAMETERS))
    .map_err(|e| WorkerError::from(AppError::new(anyhow::anyhow!("{e}"))))?;
  let initial_states = fsrs
    .next_states(None, DESIRED_RETENTION, 0)
    .map_err(|e| WorkerError::from(AppError::new(anyhow::anyhow!("{e}"))))?;
  let initial_memory = initial_states.good.memory;

  // Create EpisodicMemory with FSRS initial state
  let episodic_memory = EpisodicMemory {
    id,
    conversation_id: job.conversation_id,
    messages: job.messages,
    content: summary,
    embedding,
    stability: initial_memory.stability,
    difficulty: initial_memory.difficulty,
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
  MessageQueue::drain(job.conversation_id, messages_len, &db).await?;

  Ok(())
}
