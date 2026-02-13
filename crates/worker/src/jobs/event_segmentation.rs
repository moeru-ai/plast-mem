use std::ops::Deref;

use apalis::prelude::Data;
use chrono::Utc;
use fsrs::{DEFAULT_PARAMETERS, FSRS};
use plastmem_ai::{embed, segment_events};
use plastmem_core::{BoundaryType, EpisodicMemory, Message, MessageQueue};
use plastmem_entities::episodic_memory;
use plastmem_shared::{AppError, fsrs::DESIRED_RETENTION};
use sea_orm::{DatabaseConnection, EntityTrait};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Job for event segmentation with LLM analysis.
/// - If `check` is true: LLM decides whether to create memory
/// - If `check` is false: LLM always creates memory (forced split)
/// - `boundary_hint`: pre-determined boundary type from rule-based detection
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EventSegmentationJob {
  pub conversation_id: Uuid,
  pub messages: Vec<Message>,
  pub check: bool,
  /// Pre-determined boundary type from rule-based detection.
  pub boundary_hint: Option<BoundaryType>,
}

pub async fn process_event_segmentation(
  job: EventSegmentationJob,
  db: Data<DatabaseConnection>,
) -> Result<(), AppError> {
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

  // Call LLM for structured event segmentation analysis
  let output = segment_events(&job.messages, job.check).await?;

  // If LLM decided to skip, drain queue and return
  if output.action != "create" {
    MessageQueue::drain(job.conversation_id, job.messages.len(), &db).await?;
    return Ok(());
  }

  let summary = output.summary.unwrap_or_default();
  if summary.is_empty() {
    MessageQueue::drain(job.conversation_id, job.messages.len(), &db).await?;
    return Ok(());
  }

  let surprise = output.surprise.clamp(0.0, 1.0);

  // Determine final boundary type:
  // 1. If boundary_hint is set (e.g. TemporalGap from rule-based), use it
  // 2. If surprise > 0.7, override to PredictionError
  // 3. Otherwise, use LLM's boundary_type
  let boundary_type = if let Some(hint) = job.boundary_hint {
    hint
  } else if surprise > 0.7 {
    BoundaryType::PredictionError
  } else {
    output.boundary_type.parse::<BoundaryType>()?
  };

  let boundary_strength = surprise;

  // Generate embedding for the summary
  let embedding = embed(&summary).await?;

  let id = Uuid::now_v7();
  let now = Utc::now();
  let start_at = job.messages.first().map(|m| m.timestamp).unwrap_or(now);
  let end_at = job.messages.last().map(|m| m.timestamp).unwrap_or(now);
  let messages_len = job.messages.len();

  // Initialize FSRS state for new memory with surprise-based stability boost
  let fsrs = FSRS::new(Some(&DEFAULT_PARAMETERS))?;
  let initial_states = fsrs.next_states(None, DESIRED_RETENTION, 0)?;
  let initial_memory = initial_states.good.memory;
  let boosted_stability = initial_memory.stability * (1.0 + surprise * 0.5);

  // Create EpisodicMemory with FSRS initial state + boundary context
  let episodic_memory = EpisodicMemory {
    id,
    conversation_id: job.conversation_id,
    messages: job.messages,
    content: summary,
    embedding,
    stability: boosted_stability,
    difficulty: initial_memory.difficulty,
    surprise,
    boundary_type,
    boundary_strength,
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
    .await?;

  // Clear the processed messages from MessageQueue
  MessageQueue::drain(job.conversation_id, messages_len, &db).await?;

  Ok(())
}
