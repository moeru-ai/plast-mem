use apalis::prelude::{Data, TaskSink};
use apalis_postgres::PostgresStorage;
use chrono::Utc;
use fsrs::{DEFAULT_PARAMETERS, FSRS};
use plastmem_ai::{
  ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
  ChatCompletionRequestUserMessage, embed, generate_object,
};
use plastmem_core::{EpisodicMemory, Message, MessageQueue};
use plastmem_entities::episodic_memory;
use plastmem_shared::{AppError, fsrs::DESIRED_RETENTION};
use schemars::JsonSchema;
use sea_orm::{DatabaseConnection, EntityTrait};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::MemoryReviewJob;

/// Structured output from event segmentation LLM call.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct EventSegmentationOutput {
  /// "create" if the conversation contains significant content, "skip" if trivial
  pub action: String,
  /// Concise summary of the conversation (only when action = "create")
  pub summary: Option<String>,
  /// Prediction error / surprise score (0.0 ~ 1.0)
  /// 0 = fully expected, 1 = complete surprise
  pub surprise: f32,
}

const SEGMENT_SYSTEM_CHECK: &str = "\
You are an event segmentation analyzer. Analyze the conversation and produce a structured assessment.

1. **action**: Decide if the conversation contains significant content worth remembering.
   - \"create\" if meaningful (important information, events, decisions, or context)
   - \"skip\" if trivial (greetings, small talk, or unimportant exchanges)
   Be selective - only \"create\" if there's substantive content.

2. **summary**: If action is \"create\", provide a clear and concise summary. If \"skip\", set to null.

3. **surprise**: Rate the prediction error on a 0.0 to 1.0 scale:
   - 0.0 = fully expected, no new information
   - 0.3 = minor information gain
   - 0.7 = significant pivot or revelation
   - 1.0 = complete surprise, model-breaking";

const SEGMENT_SYSTEM_FORCE: &str = "\
You are an event segmentation analyzer. This conversation segment must be summarized (no skipping). Produce a structured assessment.

1. **action**: Always \"create\".

2. **summary**: Provide a clear and concise summary of the conversation.

3. **surprise**: Rate the prediction error on a 0.0 to 1.0 scale:
   - 0.0 = fully expected, no new information
   - 0.3 = minor information gain
   - 0.7 = significant pivot or revelation
   - 1.0 = complete surprise, model-breaking";

/// Analyzes messages for event segmentation using structured output.
///
/// When `check` is true, the LLM may return action="skip" for trivial content.
/// When `check` is false, the LLM always creates a summary.
///
/// Returns surprise score alongside the action/summary.
pub async fn segment_events(
  messages: &[Message],
  check: bool,
) -> Result<EventSegmentationOutput, AppError> {
  let system_prompt = if check {
    SEGMENT_SYSTEM_CHECK
  } else {
    SEGMENT_SYSTEM_FORCE
  };

  let messages = messages
    .iter()
    .map(std::string::ToString::to_string)
    .collect::<Vec<_>>()
    .join("\n");

  let system = ChatCompletionRequestSystemMessage::from(system_prompt);
  let user = ChatCompletionRequestUserMessage::from(messages);

  generate_object::<EventSegmentationOutput>(
    vec![
      ChatCompletionRequestMessage::System(system),
      ChatCompletionRequestMessage::User(user),
    ],
    "event_segmentation".to_owned(),
    Some("Event segmentation analysis with surprise".to_owned()),
  )
  .await
}

/// Job for event segmentation with LLM analysis.
/// - If `check` is true: LLM decides whether to create memory
/// - If `check` is false: LLM always creates memory (forced split)
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EventSegmentationJob {
  pub conversation_id: Uuid,
  pub messages: Vec<Message>,
  pub check: bool,
}

pub async fn process_event_segmentation(
  job: EventSegmentationJob,
  db: Data<DatabaseConnection>,
  review_storage: Data<PostgresStorage<MemoryReviewJob>>,
) -> Result<(), AppError> {
  let db = &*db;

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

  // If LLM decided to skip, still process pending reviews before draining
  if output.action != "create" {
    enqueue_pending_reviews(job.conversation_id, &job.messages, db, &review_storage).await?;
    MessageQueue::drain(job.conversation_id, job.messages.len(), db).await?;
    return Ok(());
  }

  let summary = output.summary.unwrap_or_default();
  if summary.is_empty() {
    enqueue_pending_reviews(job.conversation_id, &job.messages, db, &review_storage).await?;
    MessageQueue::drain(job.conversation_id, job.messages.len(), db).await?;
    return Ok(());
  }

  let surprise = output.surprise.clamp(0.0, 1.0);

  // Generate embedding for the summary
  let embedding = embed(&summary).await?;

  let id = Uuid::now_v7();
  let now = Utc::now();
  let start_at = job.messages.first().map_or(now, |m| m.timestamp);
  let end_at = job.messages.last().map_or(now, |m| m.timestamp);
  let messages_len = job.messages.len();

  // Initialize FSRS state for new memory with surprise-based stability boost
  let fsrs = FSRS::new(Some(&DEFAULT_PARAMETERS))?;
  let initial_states = fsrs.next_states(None, DESIRED_RETENTION, 0)?;
  let initial_memory = initial_states.good.memory;
  let boosted_stability = initial_memory.stability * (1.0 + surprise * 0.5);

  // Check for pending reviews and enqueue MemoryReviewJob if any
  enqueue_pending_reviews(job.conversation_id, &job.messages, db, &review_storage).await?;

  // Create EpisodicMemory with FSRS initial state
  // Surprise affects initial stability (higher surprise = longer retention)
  let episodic_memory = EpisodicMemory {
    id,
    conversation_id: job.conversation_id,
    messages: job.messages,
    content: summary,
    embedding,
    stability: boosted_stability,
    difficulty: initial_memory.difficulty,
    surprise,
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
  MessageQueue::drain(job.conversation_id, messages_len, db).await?;

  Ok(())
}

/// Take pending reviews from the queue and enqueue a MemoryReviewJob if any exist.
async fn enqueue_pending_reviews(
  conversation_id: Uuid,
  context_messages: &[Message],
  db: &DatabaseConnection,
  review_storage: &PostgresStorage<MemoryReviewJob>,
) -> Result<(), AppError> {
  if let Some(pending_reviews) = MessageQueue::take_pending_reviews(conversation_id, db).await? {
    let review_job = MemoryReviewJob {
      pending_reviews,
      context_messages: context_messages.to_vec(),
      reviewed_at: Utc::now(),
    };
    let mut storage = review_storage.clone();
    storage.push(review_job).await?;
  }
  Ok(())
}
