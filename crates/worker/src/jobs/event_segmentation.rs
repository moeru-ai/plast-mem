use apalis::prelude::{Data, TaskSink};
use apalis_postgres::PostgresStorage;
use chrono::Utc;
use fsrs::{DEFAULT_PARAMETERS, FSRS};
use plastmem_ai::{
  ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
  ChatCompletionRequestUserMessage, embed, generate_object,
};
use plastmem_core::{EpisodicMemory, Message, MessageQueue, SegmentationAction};
use plastmem_entities::episodic_memory;
use plastmem_shared::{AppError, fsrs::DESIRED_RETENTION, similarity::cosine_similarity};
use schemars::JsonSchema;
use sea_orm::{DatabaseConnection, EntityTrait, prelude::PgVector};
use serde::{Deserialize, Serialize};
use tracing::info;
use uuid::Uuid;

use super::MemoryReviewJob;

// ──────────────────────────────────────────────────
// Step 1: Boundary Detection
// ──────────────────────────────────────────────────

/// Multi-dimensional boundary signals for event boundary detection.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct BoundarySignals {
  /// Topic shift score (0.0 = same topic, 1.0 = completely different topic)
  pub topic_shift: f32,
  /// Intent shift score (0.0 = same intent, 1.0 = completely different intent)
  pub intent_shift: f32,
  /// Whether a temporal/topic transition marker was detected
  /// (e.g., "by the way", "anyway", "speaking of", "顺便说")
  pub temporal_marker: bool,
}

/// Structured output from boundary detection LLM call.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct BoundaryDetectionOutput {
  /// Whether a meaningful event boundary has been crossed
  pub is_boundary: bool,
  /// Boundary confidence score (0.0 ~ 1.0)
  pub confidence: f32,
  /// Multi-dimensional change signals
  pub signals: BoundarySignals,
  /// Updated description of "what is happening now" (when NOT a boundary)
  pub updated_event_model: Option<String>,
}

const BOUNDARY_SYSTEM_PROMPT: &str = "\
You are an event boundary detector inspired by Event Segmentation Theory. \
You maintain an internal model of \"what is happening now\" in this conversation.

Given the current event model and the conversation so far, evaluate whether \
a meaningful event boundary has been crossed with the latest message.

Evaluate boundary signals across multiple dimensions:
- **Topic coherence**: Does the latest message continue or shift the current topic?
- **Intent change**: Has the speaker's purpose changed? \
  (e.g., chatting → asking, discussing → deciding, questioning → requesting)
- **Temporal markers**: Are there phrases like \"by the way\", \"anyway\", \
  \"speaking of\", \"换个话题\", \"顺便\" that signal a topic transition?

Output:
- **is_boundary**: true if prediction error is high enough to warrant a new event
- **confidence**: how confident you are (0.0-1.0)
- **signals**: detailed scores for each dimension
- **updated_event_model**: if NOT a boundary, the updated description of what is happening now. \
  If IS a boundary, set to null.";

/// Detect whether a boundary exists, using LLM analysis.
pub async fn detect_boundary(
  messages: &[Message],
  event_model: Option<&str>,
) -> Result<BoundaryDetectionOutput, AppError> {
  let conversation = messages
    .iter()
    .map(std::string::ToString::to_string)
    .collect::<Vec<_>>()
    .join("\n");

  let user_content = if let Some(model) = event_model {
    format!(
      "Current event model: {model}\n\n\
       Conversation:\n{conversation}"
    )
  } else {
    format!("Conversation:\n{conversation}")
  };

  let system = ChatCompletionRequestSystemMessage::from(BOUNDARY_SYSTEM_PROMPT);
  let user = ChatCompletionRequestUserMessage::from(user_content);

  generate_object::<BoundaryDetectionOutput>(
    vec![
      ChatCompletionRequestMessage::System(system),
      ChatCompletionRequestMessage::User(user),
    ],
    "boundary_detection".to_owned(),
    Some("Event boundary detection with multi-dimensional signals".to_owned()),
  )
  .await
}

// ──────────────────────────────────────────────────
// Step 2: Episode Generation (Representation Alignment)
// ──────────────────────────────────────────────────

/// Structured output from episode generation LLM call.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct EpisodeGenerationOutput {
  /// Concise title capturing the episode's core theme
  pub title: String,
  /// Narrative summary of the conversation for search and retrieval
  pub summary: String,
  /// Prediction error / surprise score for the overall episode (0.0 ~ 1.0)
  /// Evaluates the information gain of this episode.
  /// 0.0 = fully expected, 1.0 = complete surprise
  pub surprise: f32,
}

const EPISODE_SYSTEM_PROMPT: &str = "\
You are an episodic memory generator. Transform this conversation segment into a structured memory.

1. **title**: A concise title (5-15 words) that captures the episode's core theme. \
   This should be descriptive and scannable.

2. **summary**: A clear, third-person narrative summarizing the conversation. \
   Preserve key facts, decisions, preferences, and context. \
   Write in a way that is useful for future retrieval via search.

3. **surprise**: Rate the information gain on a 0.0 to 1.0 scale:
   - 0.0 = fully expected, routine exchange
   - 0.3 = minor information gain
   - 0.7 = significant pivot, revelation, or decision
   - 1.0 = complete surprise, paradigm-shifting information";

/// Generate an episode (title + summary + surprise) from a segment of conversation.
pub async fn generate_episode(messages: &[Message]) -> Result<EpisodeGenerationOutput, AppError> {
  let conversation = messages
    .iter()
    .map(std::string::ToString::to_string)
    .collect::<Vec<_>>()
    .join("\n");

  let system = ChatCompletionRequestSystemMessage::from(EPISODE_SYSTEM_PROMPT);
  let user = ChatCompletionRequestUserMessage::from(conversation);

  generate_object::<EpisodeGenerationOutput>(
    vec![
      ChatCompletionRequestMessage::System(system),
      ChatCompletionRequestMessage::User(user),
    ],
    "episode_generation".to_owned(),
    Some("Episode generation with title and narrative summary".to_owned()),
  )
  .await
}

// ──────────────────────────────────────────────────
// Job definition & processing
// ──────────────────────────────────────────────────

/// Cosine similarity threshold for embedding pre-filtering.
/// Below this threshold, the LLM boundary detector is invoked.
const SIMILARITY_THRESHOLD: f32 = 0.5;

/// Boundary confidence threshold for LLM-detected boundaries.
const BOUNDARY_CONFIDENCE_THRESHOLD: f32 = 0.7;

/// Job for event segmentation with Two-Step Alignment.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EventSegmentationJob {
  pub conversation_id: Uuid,
  pub messages: Vec<Message>,
  pub action: SegmentationAction,
}

pub async fn process_event_segmentation(
  job: EventSegmentationJob,
  db: Data<DatabaseConnection>,
  review_storage: Data<PostgresStorage<MemoryReviewJob>>,
) -> Result<(), AppError> {
  let db = &*db;

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

  match job.action {
    // Force-create: skip boundary detection, go straight to episode generation.
    // Drain ALL messages (buffer overflow, no edge to preserve).
    SegmentationAction::ForceCreate => {
      info!(
        conversation_id = %job.conversation_id,
        messages = job.messages.len(),
        "Force-creating episode (buffer full)"
      );
      create_episode(&job, job.messages.len(), db, &review_storage).await?;
    }

    // Time boundary: skip boundary detection, create episode.
    // Preserve the last message for the next event (it triggered the boundary).
    SegmentationAction::TimeBoundary => {
      info!(
        conversation_id = %job.conversation_id,
        messages = job.messages.len(),
        "Creating episode (time boundary)"
      );
      // Drain all except the last one.
      // If there's only 1 message, drain_count is 0, so we do nothing.
      let drain_count = job.messages.len().saturating_sub(1);
      if drain_count > 0 {
        create_episode(&job, drain_count, db, &review_storage).await?;
      }
    }

    // Needs boundary detection with embedding pre-filter → LLM confirmation.
    SegmentationAction::NeedsBoundaryDetection => {
      let boundary_detected = check_boundary(&job, db).await?;

      if boundary_detected {
        info!(
          conversation_id = %job.conversation_id,
          messages = job.messages.len(),
          "Creating episode (boundary detected)"
        );
        let drain_count = job.messages.len().saturating_sub(1);
        if drain_count > 0 {
          create_episode(&job, drain_count, db, &review_storage).await?;
        }
      } else {
        // No boundary — just process pending reviews, don't drain.
        enqueue_pending_reviews(job.conversation_id, &job.messages, db, &review_storage).await?;
      }
    }
  }

  Ok(())
}

/// Check for a boundary using embedding similarity pre-filter + LLM confirmation.
async fn check_boundary(
  job: &EventSegmentationJob,
  db: &DatabaseConnection,
) -> Result<bool, AppError> {
  // Step 1: Embedding similarity pre-filter
  let last_embedding = MessageQueue::get_last_embedding(job.conversation_id, db).await?;

  // Compute embedding of the latest message
  let latest_msg = job
    .messages
    .last()
    .map(|m| m.content.as_str())
    .unwrap_or("");
  let new_embedding = embed(latest_msg).await?;

  if let Some(ref stored_embedding) = last_embedding {
    let similarity = cosine_similarity(stored_embedding.as_slice(), new_embedding.as_slice());
    info!(
      conversation_id = %job.conversation_id,
      similarity = similarity,
      threshold = SIMILARITY_THRESHOLD,
      "Embedding similarity pre-filter"
    );

    // High similarity = same topic, no need for LLM call
    if similarity >= SIMILARITY_THRESHOLD {
      // Update the stored embedding using rolling average to avoid drift
      let updated_vec =
        weighted_average_embedding(stored_embedding.as_slice(), new_embedding.as_slice(), 0.2);
      let new_pg_embedding = PgVector::from(updated_vec);
      MessageQueue::update_last_embedding(job.conversation_id, Some(new_pg_embedding), db).await?;
      return Ok(false);
    }
  }

  // Step 2: LLM boundary detection
  let event_model = MessageQueue::get_event_model(job.conversation_id, db).await?;
  let detection = detect_boundary(&job.messages, event_model.as_deref()).await?;

  info!(
    conversation_id = %job.conversation_id,
    is_boundary = detection.is_boundary,
    confidence = detection.confidence,
    topic_shift = detection.signals.topic_shift,
    intent_shift = detection.signals.intent_shift,
    temporal_marker = detection.signals.temporal_marker,
    "LLM boundary detection result"
  );

  let is_boundary = detection.is_boundary && detection.confidence >= BOUNDARY_CONFIDENCE_THRESHOLD;

  if !is_boundary && detection.is_boundary {
    info!(
      conversation_id = %job.conversation_id,
      confidence = detection.confidence,
      threshold = BOUNDARY_CONFIDENCE_THRESHOLD,
      "Boundary detected by LLM but confidence too low - skipping"
    );
  }

  if !is_boundary {
    // Update event model if the LLM provided one (no boundary case)
    if let Some(updated_model) = detection.updated_event_model {
      MessageQueue::update_event_model(job.conversation_id, Some(updated_model), db).await?;
    }
    // Update last embedding for next comparison (using rolling average)
    if let Some(ref stored_embedding) = last_embedding {
      let updated_vec =
        weighted_average_embedding(stored_embedding.as_slice(), new_embedding.as_slice(), 0.2);
      let pg_embedding = PgVector::from(updated_vec);
      MessageQueue::update_last_embedding(job.conversation_id, Some(pg_embedding), db).await?;
    } else {
      // Initialize if None
      let pg_embedding = PgVector::from(new_embedding);
      MessageQueue::update_last_embedding(job.conversation_id, Some(pg_embedding), db).await?;
    }
  }
  // If is_boundary is true, we do NOT update event_model or last_embedding here.
  // We proceed to create_episode, which will drain messages and initialize
  // appropriate state for the NEXT event.

  Ok(is_boundary)
}

/// Calculate weighted average of two vectors: (1 - alpha) * current + alpha * new
fn weighted_average_embedding(current: &[f32], new: &[f32], alpha: f32) -> Vec<f32> {
  if current.len() != new.len() {
    return new.to_vec();
  }

  let mut result = Vec::with_capacity(current.len());
  let mut norm = 0.0_f32;

  for (c, n) in current.iter().zip(new.iter()) {
    let val = (1.0 - alpha) * c + alpha * n;
    result.push(val);
    norm += val * val;
  }

  // Normalize
  let norm = norm.sqrt();
  if norm > 0.0 {
    for x in &mut result {
      *x /= norm;
    }
  }

  result
}

/// Create an episode from the conversation messages and drain the queue.
async fn create_episode(
  job: &EventSegmentationJob,
  drain_count: usize,
  db: &DatabaseConnection,
  review_storage: &PostgresStorage<MemoryReviewJob>,
) -> Result<(), AppError> {
  // Only generate episode from the messages being drained
  let segment_messages = &job.messages[..drain_count];

  // Step 2: Episode generation (Representation Alignment)
  let episode = generate_episode(segment_messages).await?;

  let surprise = episode.surprise.clamp(0.0, 1.0);

  if episode.summary.is_empty() {
    // Edge case: LLM returned empty summary — just drain and return
    enqueue_pending_reviews(job.conversation_id, &job.messages, db, review_storage).await?;
    MessageQueue::drain(job.conversation_id, drain_count, db).await?;
    return Ok(());
  }

  // Generate embedding for the summary
  let embedding = embed(&episode.summary).await?;

  let id = Uuid::now_v7();
  let now = Utc::now();
  let start_at = segment_messages.first().map_or(now, |m| m.timestamp);
  let end_at = segment_messages.last().map_or(now, |m| m.timestamp);

  // Initialize FSRS state with surprise-based stability boost
  let fsrs = FSRS::new(Some(&DEFAULT_PARAMETERS))?;
  let initial_states = fsrs.next_states(None, DESIRED_RETENTION, 0)?;
  let initial_memory = initial_states.good.memory;
  let boosted_stability = initial_memory.stability * (1.0 + surprise * 0.5);

  // Process pending reviews
  enqueue_pending_reviews(job.conversation_id, &job.messages, db, review_storage).await?;

  // Create EpisodicMemory with title from Two-Step Alignment
  let episodic_memory = EpisodicMemory {
    id,
    conversation_id: job.conversation_id,
    messages: segment_messages.to_vec(),
    title: episode.title,
    content: episode.summary,
    embedding: embedding.clone().into(),
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

  // Drain processed messages from MessageQueue
  MessageQueue::drain(job.conversation_id, drain_count, db).await?;

  // Update the event model for the next segment
  // (reset to None — the next boundary detection will establish a new one)
  MessageQueue::update_event_model(job.conversation_id, None, db).await?;

  // Initialize last_embedding for the NEXT event.
  // If we preserved a message (edge case), that message starts the new context.
  // If we drained everything, we reset to None to force LLM analysis on next message.
  if job.messages.len() > drain_count {
    // There is an edge message preserved in the queue
    let next_event_start_msg = &job.messages[drain_count];
    let next_embedding = embed(&next_event_start_msg.content).await?;
    let pg_embedding = PgVector::from(next_embedding);
    MessageQueue::update_last_embedding(job.conversation_id, Some(pg_embedding), db).await?;
  } else {
    // Buffer empty, reset embedding context
    // You could also use the episode summary embedding here as "past context",
    // but resetting allows the next event to establish its own identity FRESH.
    MessageQueue::update_last_embedding(job.conversation_id, None, db).await?;
  }

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
