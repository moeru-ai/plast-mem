use chrono::Utc;
use fsrs::{DEFAULT_PARAMETERS, FSRS};
use plastmem_ai::{
  ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
  ChatCompletionRequestUserMessage, embed, generate_object,
};
use plastmem_entities::episodic_memory;
use schemars::JsonSchema;
use sea_orm::{DatabaseConnection, EntityTrait, prelude::PgVector};
use serde::Deserialize;
use uuid::Uuid;

use plastmem_shared::{AppError, Message};

use crate::{EpisodicMemory, MessageQueue};

/// Desired retention rate for FSRS scheduling.
const DESIRED_RETENTION: f32 = 0.9;

/// Maximum stability boost factor from surprise signal.
/// A surprise of 1.0 yields `stability * (1 + SURPRISE_BOOST_FACTOR)`.
const SURPRISE_BOOST_FACTOR: f32 = 0.5;

// ──────────────────────────────────────────────────
// Episode Title & Summary Generation (Representation Alignment)
// ──────────────────────────────────────────────────

/// Structured output from episode generation LLM call.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct EpisodeGenerationOutput {
  /// Concise title capturing the episode's core theme
  pub title: String,
  /// Narrative summary of the conversation for search and retrieval
  pub summary: String,
}

const EPISODE_SYSTEM_PROMPT: &str = "\
You are an episodic memory generator. Transform this conversation segment into a structured memory.

1. **title**: A concise title (5-15 words) that captures the episode's core theme. \
   This should be descriptive and scannable.

2. **summary**: A clear, third-person narrative summarizing the conversation. \
   Preserve key facts, decisions, preferences, and context. \
   Write in a way that is useful for future retrieval via search.";

/// Generate episode info (title + summary) from a segment of conversation.
async fn generate_episode_info(messages: &[Message]) -> Result<EpisodeGenerationOutput, AppError> {
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
// Episode Creation
// ──────────────────────────────────────────────────

/// Info about a successfully created episode, for downstream jobs.
pub struct CreatedEpisode {
  pub id: Uuid,
  pub summary: String,
  pub messages: Vec<Message>,
  pub surprise: f32,
}

/// Create an episode from the conversation messages and drain the queue.
///
/// `next_event_embedding` is the pre-computed embedding of the message that will start
/// the next event (when there are preserved messages). This avoids redundant embedding calls.
///
/// `surprise_signal` is the embedding-based surprise (0.0 for ForceCreate/TimeBoundary).
///
/// Returns `Some(CreatedEpisode)` if an episode was created, `None` if skipped (empty summary).
pub async fn create_episode(
  conversation_id: Uuid,
  messages: &[Message],
  drain_count: usize,
  next_event_embedding: Option<PgVector>,
  surprise_signal: f32,
  db: &DatabaseConnection,
) -> Result<Option<CreatedEpisode>, AppError> {
  // Only generate episode from the messages being drained
  let segment_messages: Vec<Message> = messages[..drain_count].to_vec();

  // Episode generation (Representation Alignment)
  let episode = generate_episode_info(&segment_messages).await?;

  let surprise = surprise_signal.clamp(0.0, 1.0);

  if episode.summary.is_empty() {
    // Edge case: LLM returned empty summary — just drain and return
    MessageQueue::drain(conversation_id, drain_count, db).await?;
    return Ok(None);
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
  let boosted_stability = initial_memory.stability * (1.0 + surprise * SURPRISE_BOOST_FACTOR);

  // Create EpisodicMemory with title from Two-Step Alignment
  let episodic_memory = EpisodicMemory {
    id,
    conversation_id,
    messages: segment_messages.clone(),
    title: episode.title,
    summary: episode.summary.clone(),
    embedding: embedding,
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
  MessageQueue::drain(conversation_id, drain_count, db).await?;

  // Reset event model and its embedding for the next segment
  MessageQueue::update_event_model(conversation_id, None, db).await?;
  MessageQueue::update_event_model_embedding(conversation_id, None, db).await?;

  // Initialize last_embedding for the NEXT event.
  if messages.len() > drain_count {
    // There is an edge message preserved in the queue.
    if let Some(embedding) = next_event_embedding {
      MessageQueue::update_last_embedding(conversation_id, Some(embedding), db).await?;
    } else {
      // TimeBoundary doesn't pre-compute embedding; compute it now for the edge message.
      let next_event_start_msg = &messages[drain_count];
      let next_embedding = embed(&next_event_start_msg.content).await?;
      MessageQueue::update_last_embedding(conversation_id, Some(next_embedding), db).await?;
    }
  } else {
    // Buffer empty, reset embedding context
    MessageQueue::update_last_embedding(conversation_id, None, db).await?;
  }

  Ok(Some(CreatedEpisode {
    id,
    summary: episode.summary,
    messages: segment_messages,
    surprise,
  }))
}

