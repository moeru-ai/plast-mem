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

const FLASHBULB_SURPRISE_THRESHOLD: f32 = 0.85;
// Keep this in sync with `crates/core/src/message_queue.rs` WINDOW_MAX.
const FORCE_SINGLE_SEGMENT_QUEUE_LEN: usize = 40;
use plastmem_entities::episodic_memory;
use plastmem_shared::{APP_ENV, AppError, Message};
use schemars::JsonSchema;
use sea_orm::{DatabaseConnection, EntityTrait};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{MemoryReviewJob, PredictCalibrateJob};

// ──────────────────────────────────────────────────
// Job definition
// ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventSegmentationJob {
  pub conversation_id: Uuid,
  /// Number of messages in the queue when this job was triggered.
  pub fence_count: i32,
  /// Whether processing is forced (reached max window).
  pub force_process: bool,
  /// Whether to keep the last segment in queue for cross-window stitching.
  /// `true` for streaming ingestion, `false` for batch benchmark ingestion.
  #[serde(default = "default_keep_tail_segment")]
  pub keep_tail_segment: bool,
}

const fn default_keep_tail_segment() -> bool {
  true
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
  const fn to_signal(&self) -> f32 {
    match self {
      Self::Low => 0.2,
      Self::High => 0.6,
      Self::ExtremelyHigh => 0.9,
    }
  }
}

struct BatchSegment {
  messages: Vec<Message>,
  title: String,
  content: String,
  surprise_level: SurpriseLevel,
}

struct CreatedEpisode {
  id: Uuid,
  surprise: f32,
}

// ──────────────────────────────────────────────────
// LLM segmentation
// ──────────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
struct SegmentationPlanOutput {
  segments: Vec<SegmentationPlanItem>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SegmentationPlanItem {
  #[allow(dead_code)]
  start_message_index: u32,
  #[allow(dead_code)]
  end_message_index: u32,
  /// Authoritative field used for sequential slicing.
  num_messages: u32,
  surprise_level: SurpriseLevel,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct EpisodeContentOutput {
  title: String,
  content: String,
}

const SEGMENTATION_SYSTEM_PROMPT: &str = r#"
You are segmenting a batch of conversation messages into episodic units.
Return only JSON that matches the schema.

Create continuous, non-overlapping segments.
Start a new segment whenever there is a meaningful topic shift or a clear surprise/discontinuity.

When boundary placement is uncertain, prefer finer granularity.

# Boundary Triggers

1. **Topic & intent**
 - Meaningful changes in semantic focus, goals, or activities.
 - Subtopic transitions or shifts in user intent.
 - Explicit discourse markers that signal a transition.

2. **Surprise & discontinuity**
 - Abrupt emotional reversals or unexpected vulnerability.
 - Sudden shifts between personal/emotional and logistical/factual content.
 - Introduction of a completely new domain.
 - Sharp changes in tone, register, or notable time gaps.

- **num_messages:** Must equal the exact number of messages in the segment.
- **surprise_level:** Measure how abruptly the segment begins relative to the *preceding* segment. The first segment is `low` unless a previous episode is provided as context.
  - `low`: Gradual or routine transition.
  - `high`: Noticeable discontinuity (unexpected emotion, intent reversal, domain change).
  - `extremely_high`: Stark break (shocking event, intense emotion, major domain jump).

# Quality Constraints

- Segments must completely cover all messages exactly once, starting from index 0.
- Mathematical accuracy is strict: `num_messages` MUST equal `end_message_index - start_message_index + 1`.
- A single coherent conversation without shifts must return exactly one segment.
- Return only JSON that matches the schema."#;

const EPISODE_CONTENT_SYSTEM_PROMPT: &str = r#"
You are turning a conversation segment into an episodic memory record.
Return only JSON with `title` and `content`.

Requirements:
1. The title must be concise, descriptive, and easy to search. Keep it within 10-20 words and name the main topic, activity, or event.
2. The content must be a dated observation log, not a prose paragraph.
3. Group observations by calendar date using a `Date: Mon DD, YYYY` header.
4. Under each date, write one bullet per observation. Each bullet must begin with bracketed metadata fields, starting with `[spoken_at: ISO_8601_UTC]`.
5. Each observation should capture one specific event, statement, action, result, preference, question, intention, or state change.
6. Distinguish user assertions from questions and requests. Distinguish questions from statements of intent. Distinguish plans and possibilities from completed events.
7. Make state changes explicit when the new state replaces or updates the old one, such as `changing from`, `replacing`, or `no longer`.
8. Every observation must include a `[type: ...]` field. Use values such as `fact`, `question`, `plan`, `state_change`, `preference`, `result`, or `memory`.
9. If an observation refers to another concrete time, keep the original time phrase inside the sentence and also include `[time_expression: ...]`.
10. When that time expression can be grounded to an actual date or date range from the message timestamp, include `[referenced_time: ...]` and `[time_confidence: ...]`.
11. Use `time_confidence` values that reflect precision, such as `exact`, `exact_range`, `exact_day`, `exact_month`, `exact_year`, `estimated`, or `unresolved`.
12. Only include `referenced_time` when the expression can be grounded. Do not invent dates for vague phrases like `recently`, `soon`, `lately`, or `a while ago`.
13. If one message contains multiple events, split them into separate observation lines. Each split line must carry its own metadata fields.
14. The observation text after the metadata fields must be a complete sentence that expresses one verifiable proposition only.
15. Use actor-explicit wording. Prefer `User`, `Assistant`, or known display names over first-person retelling when the actor matters for retrieval.
16. Use precise source verbs such as `said`, `asked`, `planned`, `confirmed`, `reported`, `mentioned`, or `suggested`.
17. Do not add narrative glue such as `then`, `meanwhile`, `later`, `this led to`, or broad summary language unless it is required to preserve meaning.
18. Use precise action verbs. When the assistant clarifies vague wording and the clarification is supported, prefer the more specific verb.
19. Preserve names, places, quantities, identifiers, specific roles, and distinguishing details that make the memory searchable later.
20. Preserve unusual or user-specific phrasing in quotes when it carries important meaning.
21. Use terse, dense wording and avoid repetition. Do not invent unsupported names, places, dates, outcomes, or causal claims.

Format:
- Write one or more `Date: Mon DD, YYYY` headers as needed.
- Under each date header, write one bullet per observation using this style:
  `* [spoken_at: ISO_8601_UTC] [type: ...] [time_expression: ...] [referenced_time: ...] [time_confidence: ...] Observation text`
- Keep field names exactly as written above.
- Omit `time_expression`, `referenced_time`, and `time_confidence` when they do not apply.
- Example:
  Date: Jun 15, 2026
  * [spoken_at: 2026-06-15T09:15:00Z] [type: plan] [time_expression: this weekend] [referenced_time: 2026-06-20/2026-06-21] [time_confidence: exact_range] User said they plan to visit their parents this weekend.
  * [spoken_at: 2026-06-15T09:16:00Z] [type: question] User asked for help comparing adoption agencies.
  * [spoken_at: 2026-06-15T09:18:00Z] [type: fact] [time_expression: four years ago] [referenced_time: 2022] [time_confidence: exact_year] User said they moved from Sweden four years ago.
"#;

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

async fn generate_episode_content(
  messages: &[Message],
) -> Result<(String, String), AppError> {
  let system = ChatCompletionRequestSystemMessage::from(EPISODE_CONTENT_SYSTEM_PROMPT.trim());
  let user = ChatCompletionRequestUserMessage::from(format!(
    "Conversation segment:\n{}",
    format_messages(messages)
  ));

  let output = generate_object::<EpisodeContentOutput>(
    vec![
      ChatCompletionRequestMessage::System(system),
      ChatCompletionRequestMessage::User(user),
    ],
    "episodic_content_generation".to_owned(),
    Some("Generate episodic title and content".to_owned()),
  )
  .await?;

  let title = output.title.trim();
  let content = output.content.trim();
  Ok((
    if title.is_empty() {
      "Conversation Segment".to_owned()
    } else {
      title.to_owned()
    },
    if content.is_empty() {
      "Episode content unavailable.".to_owned()
    } else {
      content.to_owned()
    },
  ))
}

async fn batch_segment(
  messages: &[Message],
  prev_episode_content: Option<&str>,
) -> Result<Vec<BatchSegment>, AppError> {
  let formatted = format_messages(messages);

  let user_content = prev_episode_content.map_or_else(
    || format!("Messages to segment:\n{formatted}"),
    |content| {
      format!(
        "Previous episode content: {content}\n\
         Use this only as reference for the first segment's surprise level.\n\n\
         Messages to segment:\n{formatted}"
      )
    },
  );

  let system = ChatCompletionRequestSystemMessage::from(SEGMENTATION_SYSTEM_PROMPT.trim());
  let user = ChatCompletionRequestUserMessage::from(user_content);

  let output = generate_object::<SegmentationPlanOutput>(
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
      title: String::new(),
      content: String::new(),
      surprise_level: item.surprise_level,
    });
  }

  if processed_up_to < batch_len
    && let Some(last) = resolved.last_mut()
  {
    last
      .messages
      .extend_from_slice(&messages[processed_up_to..]);
    tracing::warn!(
      remaining = batch_len - processed_up_to,
      "LLM under-counted messages; absorbed into last segment"
    );
  }

  if resolved.is_empty() {
    tracing::warn!("LLM returned empty segments; treating entire batch as one segment");
    resolved.push(BatchSegment {
      messages: messages.to_vec(),
      title: String::new(),
      content: String::new(),
      surprise_level: SurpriseLevel::Low,
    });
  }

  let generated_entries = try_join_all(
    resolved
      .iter()
      .map(|segment| generate_episode_content(&segment.messages)),
  )
  .await?;

  for (segment, (title, content)) in resolved.iter_mut().zip(generated_entries) {
    segment.title = title;
    segment.content = content;
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
  content: &str,
  surprise_signal: f32,
  db: &DatabaseConnection,
) -> Result<Option<CreatedEpisode>, AppError> {
  if content.is_empty() {
    tracing::warn!(conversation_id = %conversation_id, "Skipping episode creation: empty content");
    return Ok(None);
  }

  let surprise = surprise_signal.clamp(0.0, 1.0);
  let embedding_input = if title.is_empty() {
    content.to_owned()
  } else {
    format!("{title}. {content}")
  };
  let embedding = embed(&embedding_input).await?;

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
    content: content.to_owned(),
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

  Ok(Some(CreatedEpisode { id, surprise }))
}

async fn create_episodes_batch(
  conversation_id: Uuid,
  segments: &[BatchSegment],
  db: &DatabaseConnection,
) -> Result<Vec<CreatedEpisode>, AppError> {
  let futures: Vec<_> = segments
    .iter()
    .map(|seg| {
      create_episode(
        conversation_id,
        &seg.messages,
        &seg.title,
        &seg.content,
        seg.surprise_level.to_signal(),
        db,
      )
    })
    .collect();

  let episodes: Vec<CreatedEpisode> = try_join_all(futures).await?.into_iter().flatten().collect();

  Ok(episodes)
}

// ──────────────────────────────────────────────────
// Job processing
// ──────────────────────────────────────────────────

/// Process event segmentation job.
///
/// # Panics
///
/// Panics if `to_drain` is empty when accessing the last element. This should never happen
/// because `to_drain` is created by slicing `segments` and is guaranteed to be non-empty.
pub async fn process_event_segmentation(
  job: EventSegmentationJob,
  db: Data<DatabaseConnection>,
  review_storage: Data<PostgresStorage<MemoryReviewJob>>,
  semantic_storage: Data<PostgresStorage<PredictCalibrateJob>>,
) -> Result<(), AppError> {
  let db = &*db;
  let conversation_id = job.conversation_id;
  let fence_count = usize::try_from(job.fence_count).unwrap_or(0);
  let force_process = job.force_process;
  let keep_tail_segment = job.keep_tail_segment;

  let current_messages = MessageQueue::get(conversation_id, db).await?.messages;
  let force_due_to_backlog = current_messages.len() >= FORCE_SINGLE_SEGMENT_QUEUE_LEN;
  let should_force_single_segment = force_process || force_due_to_backlog;

  // Stale job check
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
  let prev_content = MessageQueue::get_prev_episode_content(conversation_id, db).await?;
  let segments = batch_segment(batch_messages, prev_content.as_deref()).await?;

  // Single segment and not forced: defer processing and wait for more messages
  if segments.len() == 1 && !should_force_single_segment {
    tracing::info!(conversation_id = %conversation_id, "No split detected — deferring for more messages");
    MessageQueue::clear_fence(conversation_id, db).await?;
    return Ok(());
  }

  // Determine which segments to drain and the content for the next iteration
  let (drain_segments, new_prev_content): (&[BatchSegment], Option<String>) = if segments.len() == 1 {
    tracing::info!(
      conversation_id = %conversation_id,
      messages = fence_count,
      force_process = force_process,
      force_due_to_backlog = force_due_to_backlog,
      queue_len = current_messages.len(),
      "Force processing as single episode"
    );
    (&segments[..], None)
  } else if keep_tail_segment {
    let to_drain = &segments[..segments.len() - 1];
    let last_content = Some(to_drain.last().expect("non-empty").content.clone());
    tracing::info!(
      conversation_id = %conversation_id,
      total_segments = segments.len(),
      draining = to_drain.len(),
      keep_tail_segment,
      "Batch segmentation complete"
    );
    (to_drain, last_content)
  } else {
    tracing::info!(
      conversation_id = %conversation_id,
      total_segments = segments.len(),
      draining = segments.len(),
      keep_tail_segment,
      "Batch segmentation complete (drain all for batch mode)"
    );
    (&segments[..], None)
  };

  // Calculate total messages to drain
  let drain_count: usize = drain_segments.iter().map(|s| s.messages.len()).sum();

  // Enqueue pending reviews before draining
  enqueue_pending_reviews(conversation_id, batch_messages, db, &review_storage).await?;

  // Drain first (crash safety: if we crash after drain, messages are gone - acceptable loss)
  MessageQueue::drain(conversation_id, drain_count, db).await?;
  MessageQueue::finalize_job(conversation_id, new_prev_content, db).await?;

  // Then create episodes (if crash here, messages already gone - no duplicates on retry)
  let episodes = create_episodes_batch(conversation_id, drain_segments, db).await?;

  // Enqueue predict-calibrate jobs for real-time learning from each episode.
  enqueue_predict_calibrate_jobs(conversation_id, &episodes, &semantic_storage).await?;

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
  if !APP_ENV.enable_fsrs_review {
    return Ok(());
  }

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

async fn enqueue_predict_calibrate_jobs(
  conversation_id: Uuid,
  episodes: &[CreatedEpisode],
  semantic_storage: &PostgresStorage<PredictCalibrateJob>,
) -> Result<(), AppError> {
  if episodes.is_empty() {
    return Ok(());
  }

  // Enqueue jobs in parallel for better performance
  let futures: Vec<_> = episodes
    .iter()
    .map(|episode| {
      let is_flashbulb = episode.surprise >= FLASHBULB_SURPRISE_THRESHOLD;
      let job = PredictCalibrateJob {
        conversation_id,
        episode_id: episode.id,
        force: is_flashbulb,
      };
      let mut storage = semantic_storage.clone();
      async move { storage.push(job).await }
    })
    .collect();

  let results: Result<Vec<_>, _> = futures::future::join_all(futures).await.into_iter().collect();
  results?;

  tracing::info!(
    conversation_id = %conversation_id,
    created_jobs = episodes.len(),
    "Enqueued predict-calibrate jobs for new episodes"
  );

  Ok(())
}

