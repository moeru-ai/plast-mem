use std::collections::HashMap;
use std::fmt::Write;

use apalis::prelude::Data;
use chrono::{DateTime, Utc};
use fsrs::{DEFAULT_PARAMETERS, FSRS, MemoryState};
use plastmem_ai::{
  ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
  ChatCompletionRequestUserMessage, generate_object,
};
use plastmem_core::PendingReview;
use plastmem_entities::episodic_memory;
use plastmem_shared::{AppError, Message, fsrs::DESIRED_RETENTION};
use schemars::JsonSchema;
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// --- LLM Review ---

/// LLM output for memory review.
#[derive(Debug, Deserialize, JsonSchema)]
struct MemoryReviewOutput {
  pub ratings: Vec<MemoryRatingOutput>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct MemoryRatingOutput {
  /// Memory ID being reviewed
  pub memory_id: String,
  /// Rating: "again", "hard", "good", or "easy"
  pub rating: String,
}

/// FSRS rating mapped from LLM output.
#[derive(Debug, Clone, Copy)]
enum Rating {
  Again,
  Hard,
  Good,
  Easy,
}

impl Rating {
  fn parse(s: &str) -> Self {
    match s.to_lowercase().as_str() {
      "again" => Self::Again,
      "hard" => Self::Hard,
      "easy" => Self::Easy,
      _ => Self::Good,
    }
  }
}

const REVIEW_SYSTEM_PROMPT: &str = "\
You are a memory relevance reviewer. Evaluate how relevant each retrieved memory was to the conversation context.

For each memory, assign a rating:
- \"again\": Memory was not used in the conversation at all. It is noise.
- \"hard\": Memory is tangentially related but required significant inference to connect.
- \"good\": Memory is directly relevant and visibly influenced the conversation.
- \"easy\": Memory is a core pillar of the conversation. The conversation could not have proceeded meaningfully without it.

Consider:
- Whether the assistant's responses reflect knowledge from the memory
- Whether the memory's content aligns with the conversation topic
- How central the memory is to the conversation flow
- A memory matched by multiple queries may indicate higher relevance, but judge by actual usage in context";

/// Build the markdown user message for the reviewer LLM.
fn build_review_user_message(
  context_messages: &[Message],
  memories: &[(Uuid, String, Vec<String>)], // (id, summary, matched_queries)
) -> String {
  let mut out = String::new();

  let _ = writeln!(out, "## Conversation Context\n");
  for msg in context_messages {
    let _ = writeln!(out, "- {}: \"{}\"", msg.role, msg.content);
  }

  let _ = writeln!(out, "\n## Retrieved Memories\n");
  for (id, summary, queries) in memories {
    let _ = writeln!(out, "### Memory {id}");
    let _ = writeln!(out, "**Summary:** {summary}");
    let queries_str = queries
      .iter()
      .map(|q| format!("\"{q}\""))
      .collect::<Vec<_>>()
      .join(", ");
    let _ = writeln!(out, "**Matched queries:** {queries_str}");
    let _ = writeln!(out);
  }

  out
}

/// Aggregate pending reviews: deduplicate memory IDs and collect matched queries.
fn aggregate_pending_reviews(pending_reviews: &[PendingReview]) -> HashMap<Uuid, Vec<String>> {
  let mut map: HashMap<Uuid, Vec<String>> = HashMap::new();
  for review in pending_reviews {
    for id in &review.memory_ids {
      map.entry(*id).or_default().push(review.query.clone());
    }
  }
  map
}

// --- Job ---

/// Job to review retrieved memories using LLM and update FSRS parameters.
///
/// Enqueued by the event segmentation worker when pending reviews exist.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MemoryReviewJob {
  pub pending_reviews: Vec<PendingReview>,
  pub context_messages: Vec<Message>,
  pub reviewed_at: DateTime<Utc>,
}

pub async fn process_memory_review(
  job: MemoryReviewJob,
  db: Data<DatabaseConnection>,
) -> Result<(), AppError> {
  let db = &*db;

  if job.pending_reviews.is_empty() {
    return Ok(());
  }

  // 1. Aggregate: deduplicate memory IDs, collect matched queries
  let aggregated = aggregate_pending_reviews(&job.pending_reviews);

  // 2. Fetch models, apply stale + same-day filters
  let mut memories_for_review: Vec<(Uuid, String, Vec<String>)> = Vec::new();
  let mut models_by_id: HashMap<Uuid, episodic_memory::Model> = HashMap::new();

  for (memory_id, queries) in &aggregated {
    let Some(model) = episodic_memory::Entity::find_by_id(*memory_id)
      .one(db)
      .await?
    else {
      continue; // memory was deleted
    };

    let last_reviewed_at = model.last_reviewed_at.with_timezone(&Utc);
    if job.reviewed_at <= last_reviewed_at {
      continue;
    }
    if (job.reviewed_at - last_reviewed_at).num_days() < 1 {
      continue;
    }

    memories_for_review.push((*memory_id, model.summary.clone(), queries.clone()));
    models_by_id.insert(*memory_id, model);
  }

  if memories_for_review.is_empty() {
    return Ok(());
  }

  // 3. Call LLM for review
  let user_message = build_review_user_message(&job.context_messages, &memories_for_review);
  let system = ChatCompletionRequestSystemMessage::from(REVIEW_SYSTEM_PROMPT);
  let user = ChatCompletionRequestUserMessage::from(user_message);

  let output = generate_object::<MemoryReviewOutput>(
    vec![
      ChatCompletionRequestMessage::System(system),
      ChatCompletionRequestMessage::User(user),
    ],
    "memory_review".to_owned(),
    Some("Review retrieved memories for relevance".to_owned()),
  )
  .await?;

  // 4. Parse ratings and update FSRS parameters
  let fsrs = FSRS::new(Some(&DEFAULT_PARAMETERS))?;

  for rating_output in &output.ratings {
    let Ok(memory_id) = rating_output.memory_id.parse::<Uuid>() else {
      continue;
    };

    let Some(model) = models_by_id.remove(&memory_id) else {
      continue; // hallucinated ID or already processed
    };

    let last_reviewed_at = model.last_reviewed_at.with_timezone(&Utc);
    let days_elapsed = u32::try_from(
      (job.reviewed_at - last_reviewed_at).num_days().clamp(0, 365 * 100),
    )
    .unwrap_or(0);

    let current_state = MemoryState { stability: model.stability, difficulty: model.difficulty };
    let next_states = fsrs.next_states(Some(current_state), DESIRED_RETENTION, days_elapsed)?;

    let rating = Rating::parse(&rating_output.rating);
    let new_state = match rating {
      Rating::Again => next_states.again.memory,
      Rating::Hard => next_states.hard.memory,
      Rating::Good => next_states.good.memory,
      Rating::Easy => next_states.easy.memory,
    };

    let mut active_model: episodic_memory::ActiveModel = model.into();
    active_model.stability = Set(new_state.stability);
    active_model.difficulty = Set(new_state.difficulty);
    active_model.last_reviewed_at = Set(job.reviewed_at.into());
    active_model.update(db).await?;
  }

  Ok(())
}
