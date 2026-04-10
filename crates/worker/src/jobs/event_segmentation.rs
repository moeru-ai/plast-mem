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
use plastmem_event_segmentation::{
  SEGMENTATION_SYSTEM_PROMPT, SegmentationPlanOutput, SegmentedConversation, SurpriseLevel,
  build_segmentation_user_content, format_messages, resolve_segmentation_plan,
};

const FLASHBULB_SURPRISE_THRESHOLD: f32 = 0.85;
// Keep this in sync with `crates/core/src/message_queue.rs` WINDOW_MAX.
const FORCE_SINGLE_SEGMENT_QUEUE_LEN: usize = 30;
use plastmem_entities::episodic_memory;
use plastmem_shared::{APP_ENV, AppError, Message};
use schemars::JsonSchema;
use sea_orm::{DatabaseConnection, EntityTrait, TransactionTrait};
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

#[derive(Debug)]
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

struct PreparedEpisode {
  memory: plastmem_core::EpisodicMemory,
  surprise: f32,
}

// ──────────────────────────────────────────────────
// LLM segmentation
// ──────────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
struct EpisodeContentOutput {
  title: String,
  content: String,
}

const EPISODE_CONTENT_SYSTEM_PROMPT: &str = r"
You are turning a conversation segment into an episodic memory record.
Return only JSON with `title` and `content`.

Requirements:
1. The title must be concise, descriptive, and easy to search. Keep it within 10-20 words and name the main topic, activity, or event.
2. The content must be a factual observation log, like a concise case note or incident record. It is not a prose paragraph and it is not a message-by-message transcript.
3. Group observations under shared time-block headers using `At: Mon DD, YYYY h AM/PM` when the observations belong to the same approximate hour. If an hour-level header would be misleading or cannot be inferred cleanly, use `At: Mon DD, YYYY`.
4. A single `At:` header may cover multiple bullets. Do not repeat the same header for every bullet.
5. Round headers to the hour. Do not include minutes unless minute-level precision is clearly important for understanding the event.
6. Keep bullets in chronological order within each header block.
7. Each bullet should capture one dense factual observation: a concrete event, statement, action, result, preference, question, intention, or outcome. Merge adjacent turns when they belong to one coherent micro-exchange.
8. Distinguish statements, questions, requests, intentions, plans, and completed events accurately.
9. Make state changes explicit in natural language only when the change itself matters for later retrieval, such as a change in belief, feeling, plan, role, diagnosis, or outcome. Do not add a special label for state changes.
10. If an observation refers to another concrete time, keep the original time phrase in the sentence and resolve it inline in parentheses immediately after the phrase, for example `last month (July 2023)` or `the previous weekend (June 17-18, 2023)`.
11. If the time can be resolved more precisely from the message timestamps, include that grounded date, range, or hour in the parentheses. If it cannot be resolved cleanly, keep the original phrase without inventing specifics.
12. The observation text must be retrieval-friendly, specific, and source-grounded. It should read like a factual record, not a vague summary.
13. Use actor-explicit wording. Use the speaker labels that appear in the segment consistently when the actor matters for retrieval. Only fall back to generic role labels when no better speaker label is available.
14. Use precise source verbs such as `said`, `asked`, `planned`, `confirmed`, `reported`, `mentioned`, `suggested`, `showed`, or `shared`.
15. Preserve names, places, quantities, identifiers, specific roles, and distinguishing details that make the memory searchable later.
16. Preserve nicknames, short forms, product or game titles, place names, titles of works, device names, diagnosis-like wording, and other distinctive wording verbatim when they may matter for later retrieval or QA.
17. Do not replace a specific original term with a fuller, broader, or more generic paraphrase if the original wording is supported by the conversation.
18. When possible, bind the time, place, actor, and event in the same bullet instead of scattering them across separate lines.
19. Prefer compression over repetition, but do not compress away named entities, rare lexical clues, exact labels, grounded time references, or concrete formulations that may be directly asked about later.
20. Use terse, dense wording. Avoid filler, narrative glue, broad moralizing summaries, and unsupported causal claims.
21. Do not use any bracketed metadata tags such as `[spoken_at: ...]`, `[type: ...]`, `[time_expression: ...]`, `[referenced_time: ...]`, or `[time_confidence: ...]`.

Format:
- Write one or more shared `At:` headers as needed.
- Under each header, write one bullet per observation using this style:
  `* Observation text`
- Example:
  At: Jun 15, 2026 3 PM
  * Sam said he planned to visit his parents that weekend (June 20-21, 2026).
  * Evan asked for help comparing adoption agencies.
  * Sam said he moved from Sweden four years earlier (2022).
";

async fn request_segmentation_plan(
  messages: &[Message],
  prev_episode_content: Option<&str>,
  retry_reason: Option<&str>,
) -> Result<SegmentationPlanOutput, AppError> {
  let system = ChatCompletionRequestSystemMessage::from(SEGMENTATION_SYSTEM_PROMPT.trim());
  let user = ChatCompletionRequestUserMessage::from(build_segmentation_user_content(
    messages,
    prev_episode_content,
    retry_reason,
  ));

  generate_object::<SegmentationPlanOutput>(
    vec![
      ChatCompletionRequestMessage::System(system),
      ChatCompletionRequestMessage::User(user),
    ],
    "batch_segmentation".to_owned(),
    Some("Batch episodic memory segmentation".to_owned()),
  )
  .await
}

async fn generate_episode_content(messages: &[Message]) -> Result<(String, String), AppError> {
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
  let mut retry_reason: Option<String> = None;
  let mut resolved = None;

  for attempt in 1..=2 {
    let output = match request_segmentation_plan(
      messages,
      prev_episode_content,
      retry_reason.as_deref(),
    )
    .await
    {
      Ok(output) => output,
      Err(err) => {
        if attempt == 2 {
          return Err(err);
        }

        let reason = format!("first attempt failed before producing a plan: {err}");
        tracing::warn!(attempt, reason = %reason, "Segmentation request failed; retrying");
        retry_reason = Some(reason);
        continue;
      }
    };

    resolved = Some(
      resolve_segmentation_plan(messages, output.segments)
        .map_err(|reason| AppError::new(anyhow::anyhow!(reason)))?,
    );
    break;
  }

  let resolved = resolved.expect("segmentation loop must either resolve or return");
  let mut resolved: Vec<BatchSegment> = resolved
    .into_iter()
    .map(|segment: SegmentedConversation| BatchSegment {
      messages: segment.messages,
      title: String::new(),
      content: String::new(),
      surprise_level: segment.surprise_level,
    })
    .collect();

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

async fn prepare_episode(
  conversation_id: Uuid,
  messages: &[Message],
  title: &str,
  content: &str,
  surprise_signal: f32,
) -> Result<Option<PreparedEpisode>, AppError> {
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

  Ok(Some(PreparedEpisode {
    memory: mem,
    surprise,
  }))
}

async fn prepare_episodes_batch(
  conversation_id: Uuid,
  segments: &[BatchSegment],
) -> Result<Vec<PreparedEpisode>, AppError> {
  let futures: Vec<_> = segments
    .iter()
    .map(|seg| {
      prepare_episode(
        conversation_id,
        &seg.messages,
        &seg.title,
        &seg.content,
        seg.surprise_level.to_signal(),
      )
    })
    .collect();

  let episodes: Vec<PreparedEpisode> = try_join_all(futures).await?.into_iter().flatten().collect();

  Ok(episodes)
}

async fn persist_episodes_batch(
  conversation_id: Uuid,
  drain_count: usize,
  prev_episode_content: Option<String>,
  episodes: &[PreparedEpisode],
  db: &DatabaseConnection,
) -> Result<Vec<CreatedEpisode>, AppError> {
  let txn = db.begin().await?;

  let active_models: Vec<episodic_memory::ActiveModel> = episodes
    .iter()
    .map(|episode| {
      let model = episode.memory.to_model()?;
      Ok::<_, AppError>(model.into())
    })
    .collect::<Result<_, _>>()?;

  if !active_models.is_empty() {
    episodic_memory::Entity::insert_many(active_models)
      .exec(&txn)
      .await?;
  }

  MessageQueue::drain(conversation_id, drain_count, &txn).await?;
  MessageQueue::finalize_job(conversation_id, prev_episode_content, &txn).await?;
  txn.commit().await?;

  let created = episodes
    .iter()
    .map(|episode| {
      tracing::info!(
        episode_id = %episode.memory.id,
        conversation_id = %conversation_id,
        title = %episode.memory.title,
        messages = episode.memory.messages.len(),
        surprise = episode.surprise,
        "Episode created"
      );

      CreatedEpisode {
        id: episode.memory.id,
        surprise: episode.surprise,
      }
    })
    .collect();

  Ok(created)
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
  let (drain_segments, new_prev_content): (&[BatchSegment], Option<String>) = if segments.len() == 1
  {
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

  // Enqueue pending reviews before persistence so a failure leaves the queue untouched.
  enqueue_pending_reviews(conversation_id, batch_messages, db, &review_storage).await?;

  let prepared_episodes = prepare_episodes_batch(conversation_id, drain_segments).await?;
  let episodes = persist_episodes_batch(
    conversation_id,
    drain_count,
    new_prev_content,
    &prepared_episodes,
    db,
  )
  .await?;

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

  let results: Result<Vec<_>, _> = futures::future::join_all(futures)
    .await
    .into_iter()
    .collect();
  results?;

  tracing::info!(
    conversation_id = %conversation_id,
    created_jobs = episodes.len(),
    "Enqueued predict-calibrate jobs for new episodes"
  );

  Ok(())
}
