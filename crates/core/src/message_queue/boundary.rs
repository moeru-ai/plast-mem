use plastmem_ai::{
  ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
  ChatCompletionRequestUserMessage, cosine_similarity, embed, generate_object,
};
use plastmem_shared::{AppError, Message};
use schemars::JsonSchema;
use sea_orm::{DatabaseConnection, prelude::PgVector};
use serde::Deserialize;
use tracing::info;
use uuid::Uuid;

use super::MessageQueue;

/// Topic channel: cosine similarity threshold for embedding pre-filtering.
/// Below this threshold, the LLM boundary detector is invoked.
const TOPIC_SIMILARITY_THRESHOLD: f32 = 0.5;

/// Surprise channel: surprise signal threshold.
/// Surprise = 1 - `cosine_similarity(event_model`, `new_message`).
/// Above this threshold (high prediction error), a boundary is triggered directly without LLM.
const SURPRISE_THRESHOLD: f32 = 0.7;

/// Weight for new embeddings in the rolling average update.
/// `(1 - alpha) * current + alpha * new`
const EMBEDDING_ROLLING_ALPHA: f32 = 0.2;

// ──────────────────────────────────────────────────
// LLM Boundary Detection
// ──────────────────────────────────────────────────

/// Structured output from boundary detection LLM call.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct BoundaryDetectionOutput {
  /// Whether a meaningful event boundary has been crossed
  pub is_boundary: bool,
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
- **is_boundary**: true if a meaningful event boundary has been crossed
- **updated_event_model**: if NOT a boundary, the updated description of what is happening now. \
  If IS a boundary, set to null.";

/// Detect topic shift using LLM analysis.
async fn llm_topic_shift_detect(
  messages: &[Message],
  event_model: Option<&str>,
) -> Result<BoundaryDetectionOutput, AppError> {
  let conversation = messages
    .iter()
    .map(std::string::ToString::to_string)
    .collect::<Vec<_>>()
    .join("\n");

  let user_content = event_model.map_or_else(
    || format!("Conversation:\n{conversation}"),
    |model| {
      format!(
        "Current event model: {model}\n\n\
         Conversation:\n{conversation}"
      )
    },
  );

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
// Dual-channel boundary check
// ──────────────────────────────────────────────────

/// Result of dual-channel boundary detection.
pub struct BoundaryResult {
  /// Whether a boundary was detected (topic channel OR surprise channel).
  pub is_boundary: bool,
  /// Pre-computed embedding of the latest message (reused by `create_episode`).
  pub latest_embedding: Option<PgVector>,
  /// Surprise signal: `1 - cosine_sim(event_model_embedding, new_embedding)`.
  /// 0.0 if `event_model_embedding` is not available.
  pub surprise_signal: f32,
}

/// Check for a boundary using dual-channel detection:
/// - **Topic channel**: embedding similarity pre-filter → LLM confirmation
/// - **Surprise channel**: event model embedding divergence → direct boundary
///
/// Either channel triggering results in a boundary (OR relationship).
pub async fn detect_boundary(
  conversation_id: Uuid,
  messages: &[Message],
  db: &DatabaseConnection,
) -> Result<BoundaryResult, AppError> {
  let last_embedding = MessageQueue::get_last_embedding(conversation_id, db).await?;
  let event_model_embedding = MessageQueue::get_event_model_embedding(conversation_id, db).await?;

  // Compute embedding of the latest message
  let latest_msg = messages.last().map_or("", |m| m.content.as_str());
  let new_embedding = embed(latest_msg).await?;

  // === Surprise channel ===
  // Compute surprise signal regardless of topic channel outcome.
  let surprise_signal = event_model_embedding.as_ref().map_or(0.0, |em_embedding| {
    let sim = cosine_similarity(em_embedding.as_slice(), new_embedding.as_slice());
    let surprise = 1.0 - sim;
    info!(
      conversation_id = %conversation_id,
      similarity = sim,
      surprise = surprise,
      threshold = SURPRISE_THRESHOLD,
      "Surprise channel"
    );
    surprise
  });

  let surprise_boundary = surprise_signal > SURPRISE_THRESHOLD;

  if surprise_boundary {
    info!(
      conversation_id = %conversation_id,
      surprise_signal = surprise_signal,
      "Surprise channel triggered direct boundary"
    );
    return Ok(BoundaryResult {
      is_boundary: true,
      latest_embedding: Some(new_embedding),
      surprise_signal,
    });
  }

  // === Topic channel ===
  if let Some(ref stored_embedding) = last_embedding {
    let similarity = cosine_similarity(stored_embedding.as_slice(), new_embedding.as_slice());
    info!(
      conversation_id = %conversation_id,
      similarity = similarity,
      threshold = TOPIC_SIMILARITY_THRESHOLD,
      "Topic channel: embedding similarity pre-filter"
    );

    // High similarity = same topic, no need for LLM call
    if similarity >= TOPIC_SIMILARITY_THRESHOLD {
      // Update the stored embedding using rolling average to avoid drift
      let updated_vec = weighted_average_embedding(
        stored_embedding.as_slice(),
        new_embedding.as_slice(),
        EMBEDDING_ROLLING_ALPHA,
      );
      let updated_embedding = PgVector::from(updated_vec);
      MessageQueue::update_last_embedding(conversation_id, Some(updated_embedding), db).await?;
      return Ok(BoundaryResult {
        is_boundary: false,
        latest_embedding: Some(new_embedding),
        surprise_signal,
      });
    }
  }

  // Topic channel: LLM boundary detection
  let event_model = MessageQueue::get_event_model(conversation_id, db).await?;
  let detection = llm_topic_shift_detect(messages, event_model.as_deref()).await?;

  info!(
    conversation_id = %conversation_id,
    is_boundary = detection.is_boundary,
    "Topic channel: LLM boundary detection result"
  );

  let is_boundary = detection.is_boundary;

  if !is_boundary {
    // Update event model if the LLM provided one (no boundary case)
    if let Some(ref updated_model) = detection.updated_event_model {
      MessageQueue::update_event_model(conversation_id, Some(updated_model.clone()), db).await?;
      // Sync event_model_embedding for surprise channel
      let model_embedding = embed(updated_model).await?;
      MessageQueue::update_event_model_embedding(conversation_id, Some(model_embedding), db)
        .await?;
    }
    // Update last embedding for next comparison (using rolling average)
    if let Some(ref stored_embedding) = last_embedding {
      let updated_vec = weighted_average_embedding(
        stored_embedding.as_slice(),
        new_embedding.as_slice(),
        EMBEDDING_ROLLING_ALPHA,
      );
      let updated_embedding = PgVector::from(updated_vec);
      MessageQueue::update_last_embedding(conversation_id, Some(updated_embedding), db).await?;
    } else {
      // Initialize if None
      MessageQueue::update_last_embedding(conversation_id, Some(new_embedding.clone()), db).await?;
    }
  }

  Ok(BoundaryResult {
    is_boundary,
    latest_embedding: Some(new_embedding),
    surprise_signal,
  })
}

/// Calculate weighted average of two vectors: (1 - alpha) * current + alpha * new
fn weighted_average_embedding(current: &[f32], new: &[f32], alpha: f32) -> Vec<f32> {
  debug_assert_eq!(
    current.len(),
    new.len(),
    "Embedding dimensions must match for weighted average."
  );
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
  if norm > 1e-9 {
    let inv_norm = 1.0 / norm;
    for x in &mut result {
      *x *= inv_norm;
    }
  }

  result
}
