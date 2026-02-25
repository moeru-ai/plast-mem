use apalis::prelude::{Data, TaskSink};
use apalis_postgres::PostgresStorage;
use chrono::Utc;
use fsrs::{DEFAULT_PARAMETERS, FSRS};
use futures::future::try_join_all;
use plastmem_ai::{
  ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
  ChatCompletionRequestUserMessage, embed, generate_object,
};
use plastmem_core::MessageQueue;

const CONSOLIDATION_EPISODE_THRESHOLD: u64 = 3;
const FLASHBULB_SURPRISE_THRESHOLD: f32 = 0.85;
use plastmem_entities::episodic_memory;
use plastmem_shared::{AppError, Message};
use schemars::JsonSchema;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{MemoryReviewJob, SemanticConsolidationJob};

// ──────────────────────────────────────────────────
// Job definition
// ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventSegmentationJob {
  pub conversation_id: Uuid,
  /// Number of messages in the queue when this job was triggered.
  pub fence_count: i32,
}

// ──────────────────────────────────────────────────
// Segmentation types
// ──────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum SurpriseLevel {
  Low,
  High,
  ExtremelyHigh,
}

impl SurpriseLevel {
  fn to_signal(&self) -> f32 {
    match self {
      SurpriseLevel::Low => 0.2,
      SurpriseLevel::High => 0.6,
      SurpriseLevel::ExtremelyHigh => 0.9,
    }
  }
}

struct BatchSegment {
  messages: Vec<Message>,
  title: String,
  summary: String,
  surprise_level: SurpriseLevel,
}

struct CreatedEpisode {
  surprise: f32,
}

// ──────────────────────────────────────────────────
// LLM segmentation
// ──────────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
struct SegmentationOutput {
  segments: Vec<SegmentItem>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SegmentItem {
  #[allow(dead_code)]
  start_message_index: u32,
  #[allow(dead_code)]
  end_message_index: u32,
  /// Authoritative field used for sequential slicing.
  num_messages: u32,
  title: String,
  summary: String,
  surprise_level: SurpriseLevel,
}

const SEGMENTATION_SYSTEM_PROMPT: &str = "\
# Instruction

You are an episodic memory segmentation system (Event Segmentation Theory).
Segment the conversation into episodes — moments where the current event model no longer predicts the next turn.

A segment boundary MUST be created when **either**:
- The topic or intent changes meaningfully, OR
- The new message is surprising or discontinuous relative to prior context.

When in doubt, split rather than merge.

---

# Output Format

Return a JSON list of segments. Each segment must include:
- `start_message_index` — 0-indexed position of the first message (inclusive)
- `end_message_index` — 0-indexed position of the last message (inclusive)
- `num_messages` — number of messages; must equal `end_message_index − start_message_index + 1`
- `title` — 5–15 words capturing the core theme
- `summary` — ≤50 words, third-person narrative (e.g., \"The user asked X; the assistant explained Y…\")
- `surprise_level` — `low` | `high` | `extremely_high` (relative to the preceding segment)

---

# Segmentation Rules

## 1. Topic-Aware Rules
- Group consecutive messages sharing the same semantic focus, goal, or activity.
- A boundary occurs when subject matter, intent, or activity changes meaningfully.
- Subtopic changes count (e.g., emotional support → career advice → casual chat).
- Watch for discourse markers: \"by the way\", \"anyway\", \"换个话题\", \"对了\" — these signal deliberate transitions.
- Intent shifts count: chatting→deciding, venting→requesting help.

## 2. Surprise-Aware Rules
Create a boundary if a message diverges abruptly from prior context:
- Sudden emotional reversal or unexpected vulnerability
- Shift between personal/emotional and logistical/factual content
- Introduction of a new domain (health, work, relationships, finance, etc.)
- Sharp change in tone, register, or a notable time gap (visible in timestamps)

## 3. Fusion Policy
- A boundary is created if **either** channel triggers — topic shift OR surprise.
- Prefer finer granularity: when in doubt, split rather than merge.

## 4. Surprise Level
Measures how abruptly this segment begins relative to the preceding segment:
- `low` — gradual or routine transition
- `high` — noticeable discontinuity: unexpected emotion, intent reversal, or domain change
- `extremely_high` — stark break: shocking event, intense emotion, or major domain jump

First segment: assess relative to the previous episode summary if provided; otherwise use `low`.

---

# Quality Requirements
- Segments must be consecutive, non-overlapping, and cover all messages exactly once.
- The first segment must start at message index 0.
- `num_messages` is the authoritative field for slicing and must be accurate.
- All `num_messages` values must sum to the total input message count.
- A single coherent conversation must return exactly one segment covering all messages.";

fn format_messages(messages: &[Message]) -> String {
  messages
    .iter()
    .enumerate()
    .map(|(i, m)| {
      format!(
        "[{}] {} [{}] {}",
        i,
        m.timestamp.format("%Y-%m-%dT%H:%M:%SZ"),
        m.role,
        m.content
      )
    })
    .collect::<Vec<_>>()
    .join("\n")
}

async fn batch_segment(
  messages: &[Message],
  prev_episode_summary: Option<&str>,
) -> Result<Vec<BatchSegment>, AppError> {
  let formatted = format_messages(messages);

  let user_content = match prev_episode_summary {
    Some(summary) => format!(
      "Previous episode: {summary}\n\
       Use this as the reference point for the first segment's surprise_level.\n\n\
       Messages to segment:\n{formatted}"
    ),
    None => format!("Messages to segment:\n{formatted}"),
  };

  let system = ChatCompletionRequestSystemMessage::from(SEGMENTATION_SYSTEM_PROMPT);
  let user = ChatCompletionRequestUserMessage::from(user_content);

  let output = generate_object::<SegmentationOutput>(
    vec![
      ChatCompletionRequestMessage::System(system),
      ChatCompletionRequestMessage::User(user),
    ],
    "batch_segmentation".to_owned(),
    Some("Batch episodic memory segmentation".to_owned()),
  )
  .await?;

  let batch_len = messages.len();
  let mut resolved = Vec::with_capacity(output.segments.len());
  let mut processed_up_to: usize = 0;

  for (i, item) in output.segments.into_iter().enumerate() {
    let start = processed_up_to;
    let count = item.num_messages as usize;
    let end = (start + count).min(batch_len);

    if start >= batch_len {
      tracing::warn!(
        segment_idx = i,
        batch_len,
        start,
        "LLM segment out of bounds, skipping"
      );
      break;
    }

    processed_up_to = end;
    resolved.push(BatchSegment {
      messages: messages[start..end].to_vec(),
      title: item.title,
      summary: item.summary,
      surprise_level: item.surprise_level,
    });
  }

  if processed_up_to < batch_len {
    if let Some(last) = resolved.last_mut() {
      last
        .messages
        .extend_from_slice(&messages[processed_up_to..]);
      tracing::warn!(
        remaining = batch_len - processed_up_to,
        "LLM under-counted messages; absorbed into last segment"
      );
    }
  }

  if resolved.is_empty() {
    tracing::warn!("LLM returned empty segments; treating entire batch as one segment");
    resolved.push(BatchSegment {
      messages: messages.to_vec(),
      title: "Conversation Segment".to_owned(),
      summary: "Conversation summary unavailable (segmentation fallback).".to_owned(),
      surprise_level: SurpriseLevel::Low,
    });
  }

  Ok(resolved)
}

// ──────────────────────────────────────────────────
// Episode creation
// ──────────────────────────────────────────────────

const DESIRED_RETENTION: f32 = 0.9;
const SURPRISE_BOOST_FACTOR: f32 = 0.5;

async fn create_episode(
  conversation_id: Uuid,
  messages: &[Message],
  title: &str,
  summary: &str,
  surprise_signal: f32,
  db: &DatabaseConnection,
) -> Result<Option<CreatedEpisode>, AppError> {
  if summary.is_empty() {
    tracing::warn!(conversation_id = %conversation_id, "Skipping episode creation: empty summary");
    return Ok(None);
  }

  let surprise = surprise_signal.clamp(0.0, 1.0);
  let embedding = embed(summary).await?;

  let id = Uuid::now_v7();
  let now = Utc::now();
  let start_at = messages.first().map_or(now, |m| m.timestamp);
  let end_at = messages.last().map_or(now, |m| m.timestamp);

  let fsrs = FSRS::new(Some(&DEFAULT_PARAMETERS))?;
  let initial_states = fsrs.next_states(None, DESIRED_RETENTION, 0)?;
  let initial_state = initial_states.good.memory;
  let boosted_stability = initial_state.stability * (1.0 + surprise * SURPRISE_BOOST_FACTOR);

  let mem = plastmem_core::EpisodicMemory {
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

  let model = mem.to_model()?;
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
    "Episode created"
  );

  Ok(Some(CreatedEpisode { surprise }))
}

// ──────────────────────────────────────────────────
// Job processing
// ──────────────────────────────────────────────────

pub async fn process_event_segmentation(
  job: EventSegmentationJob,
  db: Data<DatabaseConnection>,
  review_storage: Data<PostgresStorage<MemoryReviewJob>>,
  semantic_storage: Data<PostgresStorage<SemanticConsolidationJob>>,
) -> Result<(), AppError> {
  let db = &*db;
  let review_storage = &*review_storage;
  let semantic_storage = &*semantic_storage;
  let conversation_id = job.conversation_id;
  let fence_count = job.fence_count as usize;

  let current_messages = MessageQueue::get(conversation_id, db).await?.messages;
  let window_doubled = MessageQueue::get_or_create_model(conversation_id, db)
    .await?
    .window_doubled;

  if current_messages.len() < fence_count {
    tracing::debug!(
      conversation_id = %conversation_id,
      fence_count,
      actual = current_messages.len(),
      "Stale event segmentation job — clearing fence"
    );
    MessageQueue::finalize_job(conversation_id, None, db).await?;
    return Ok(());
  }

  let batch_messages = &current_messages[..fence_count];

  tracing::debug!(
    conversation_id = %conversation_id,
    fence_count,
    window_doubled,
    "Processing batch segmentation"
  );

  let prev_summary = MessageQueue::get_prev_episode_summary(conversation_id, db).await?;
  let segments = batch_segment(batch_messages, prev_summary.as_deref()).await?;

  match segments.len() {
    // ── No split ──────────────────────────────────────────────────────────
    1 if !window_doubled => {
      tracing::info!(conversation_id = %conversation_id, "No split detected — doubling window");
      MessageQueue::set_doubled_and_clear_fence(conversation_id, db).await?;
    }

    // ── No split after doubling: force drain ──────────────────────────────
    1 => {
      tracing::info!(
        conversation_id = %conversation_id,
        messages = fence_count,
        "No split after doubled window — force draining as single episode"
      );
      let seg = &segments[0];

      MessageQueue::drain(conversation_id, fence_count, db).await?;
      MessageQueue::finalize_job(conversation_id, None, db).await?;

      enqueue_pending_reviews(conversation_id, batch_messages, db, review_storage).await?;

      if let Some(episode) = create_episode(
        conversation_id,
        &seg.messages,
        &seg.title,
        &seg.summary,
        seg.surprise_level.to_signal(),
        db,
      )
      .await?
      {
        enqueue_semantic_consolidation(conversation_id, episode, db, semantic_storage).await?;
      }
    }

    // ── Multiple segments: drain all but last ─────────────────────────────
    _ => {
      let drain_segments = &segments[..segments.len() - 1];

      tracing::info!(
        conversation_id = %conversation_id,
        total_segments = segments.len(),
        draining = drain_segments.len(),
        "Batch segmentation complete"
      );

      let drain_count: usize = drain_segments.iter().map(|s| s.messages.len()).sum();
      let new_prev_summary = Some(drain_segments.last().expect("non-empty").summary.clone());
      MessageQueue::drain(conversation_id, drain_count, db).await?;
      MessageQueue::finalize_job(conversation_id, new_prev_summary, db).await?;

      enqueue_pending_reviews(conversation_id, batch_messages, db, review_storage).await?;

      let episode_futures: Vec<_> = drain_segments
        .iter()
        .map(|seg| {
          create_episode(
            conversation_id,
            &seg.messages,
            &seg.title,
            &seg.summary,
            seg.surprise_level.to_signal(),
            db,
          )
        })
        .collect();

      let created_episodes: Vec<CreatedEpisode> = try_join_all(episode_futures)
        .await?
        .into_iter()
        .flatten()
        .collect();

      for episode in created_episodes {
        enqueue_semantic_consolidation(conversation_id, episode, db, semantic_storage).await?;
      }

    }
  }

  Ok(())
}

// ──────────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────────

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

async fn enqueue_semantic_consolidation(
  conversation_id: Uuid,
  episode: CreatedEpisode,
  db: &DatabaseConnection,
  semantic_storage: &PostgresStorage<SemanticConsolidationJob>,
) -> Result<(), AppError> {
  let is_flashbulb = episode.surprise >= FLASHBULB_SURPRISE_THRESHOLD;
  let unconsolidated_count = count_unconsolidated(conversation_id, db).await?;
  let threshold_reached = unconsolidated_count >= CONSOLIDATION_EPISODE_THRESHOLD;

  if is_flashbulb || threshold_reached {
    let job = SemanticConsolidationJob {
      conversation_id,
      force: is_flashbulb,
    };
    let mut storage = semantic_storage.clone();
    storage.push(job).await?;
    tracing::info!(
      conversation_id = %conversation_id,
      unconsolidated_count,
      is_flashbulb,
      "Enqueued semantic consolidation job"
    );
  } else {
    tracing::debug!(
      conversation_id = %conversation_id,
      unconsolidated_count,
      "Accumulating episode for later consolidation"
    );
  }

  Ok(())
}

async fn count_unconsolidated(
  conversation_id: Uuid,
  db: &DatabaseConnection,
) -> Result<u64, AppError> {
  let count = episodic_memory::Entity::find()
    .filter(episodic_memory::Column::ConsolidatedAt.is_null())
    .filter(episodic_memory::Column::ConversationId.eq(conversation_id))
    .count(db)
    .await?;
  Ok(count)
}
