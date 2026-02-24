use chrono::Utc;
use fsrs::{DEFAULT_PARAMETERS, FSRS};
use plastmem_ai::embed;
use plastmem_entities::episodic_memory;
use sea_orm::{DatabaseConnection, EntityTrait};
use uuid::Uuid;

use plastmem_shared::{AppError, Message};

use crate::EpisodicMemory;

/// Desired retention rate for FSRS scheduling.
const DESIRED_RETENTION: f32 = 0.9;

/// Maximum stability boost factor from surprise signal.
/// A surprise of 1.0 yields `stability * (1 + SURPRISE_BOOST_FACTOR)`.
const SURPRISE_BOOST_FACTOR: f32 = 0.5;

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

/// Create an episodic memory record from a pre-segmented batch segment.
///
/// Title and summary are provided by the batch segmentation LLM call — no additional
/// LLM call is needed here. Does NOT drain the message queue (caller is responsible).
///
/// Returns `Some(CreatedEpisode)` on success, `None` if summary is empty (skip).
pub async fn create_episode_from_segment(
  conversation_id: Uuid,
  messages: &[Message],
  title: &str,
  summary: &str,
  surprise_signal: f32,
  db: &DatabaseConnection,
) -> Result<Option<CreatedEpisode>, AppError> {
  if summary.is_empty() {
    tracing::warn!(
      conversation_id = %conversation_id,
      "Skipping episode creation: empty summary"
    );
    return Ok(None);
  }

  let surprise = surprise_signal.clamp(0.0, 1.0);

  // Embed the summary for retrieval
  let embedding = embed(summary).await?;

  let id = Uuid::now_v7();
  let now = Utc::now();
  let start_at = messages.first().map_or(now, |m| m.timestamp);
  let end_at = messages.last().map_or(now, |m| m.timestamp);

  // Initialize FSRS state with surprise-based stability boost
  let fsrs = FSRS::new(Some(&DEFAULT_PARAMETERS))?;
  let initial_states = fsrs.next_states(None, DESIRED_RETENTION, 0)?;
  let initial_state = initial_states.good.memory;
  let boosted_stability = initial_state.stability * (1.0 + surprise * SURPRISE_BOOST_FACTOR);

  let episodic_memory = EpisodicMemory {
    id,
    conversation_id,
    messages: messages.to_vec(),
    title: title.to_owned(),
    summary: summary.to_owned(),
    embedding,
    stability: boosted_stability,
    difficulty: initial_state.difficulty,
    surprise,
    start_at,
    end_at,
    created_at: now,
    last_reviewed_at: now,
    consolidated_at: None,
  };

  let model = episodic_memory.to_model()?;
  let active_model: episodic_memory::ActiveModel = model.into();
  episodic_memory::Entity::insert(active_model)
    .exec(db)
    .await?;

  tracing::info!(
    episode_id = %id,
    conversation_id = %conversation_id,
    title = %title,
    messages = messages.len(),
    surprise,
    "Episode created from batch segment"
  );

  Ok(Some(CreatedEpisode {
    id,
    summary: summary.to_owned(),
    messages: messages.to_vec(),
    surprise,
  }))
}
