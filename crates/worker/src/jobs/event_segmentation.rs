use apalis::prelude::{Data, TaskSink};
use apalis_postgres::PostgresStorage;
use chrono::Utc;
use futures::future::try_join_all;
use plastmem_core::{
  CONSOLIDATION_EPISODE_THRESHOLD, EpisodicMemory, FLASHBULB_SURPRISE_THRESHOLD, MessageQueue,
  batch_segment, create_episode_from_segment,
};
use plastmem_shared::{AppError, Message};
use sea_orm::DatabaseConnection;
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
  /// The job processes messages[0..fence_count].
  pub fence_count: i32,
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

  // Fetch current queue state
  let current_messages = MessageQueue::get(conversation_id, db).await?.messages;
  let window_doubled = MessageQueue::get_or_create_model(conversation_id, db)
    .await?
    .window_doubled;

  // Validate fence: if queue has fewer messages than fence_count the job is stale
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

  // Fetch previous episode summary for first-segment surprise context
  let prev_summary = MessageQueue::get_prev_episode_summary(conversation_id, db).await?;

  // Run batch LLM segmentation
  let segments =
    batch_segment(batch_messages, prev_summary.as_deref()).await?;

  match segments.len() {
    // ── No split ──────────────────────────────────────────────────────────
    1 if !window_doubled => {
      // First time returning 1 segment: double the window and wait for more messages
      tracing::info!(
        conversation_id = %conversation_id,
        "No split detected — doubling window"
      );
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

      // Drain and finalize BEFORE episode creation.
      // If creation fails or crashes afterward, messages are already gone — acceptable data loss.
      MessageQueue::drain(conversation_id, fence_count, db).await?;
      MessageQueue::finalize_job(conversation_id, None, db).await?;

      enqueue_pending_reviews(conversation_id, batch_messages, db, review_storage).await?;

      if let Some(episode) = create_episode_from_segment(
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
      let last_segment = segments.last().expect("segments non-empty");

      tracing::info!(
        conversation_id = %conversation_id,
        total_segments = segments.len(),
        draining = drain_segments.len(),
        "Batch segmentation complete"
      );

      // Drain and finalize BEFORE episode creation.
      // If creation fails or crashes afterward, messages are already gone — acceptable data loss.
      let drain_count: usize = drain_segments.iter().map(|s| s.messages.len()).sum();
      let new_prev_summary = Some(drain_segments.last().expect("non-empty").summary.clone());
      MessageQueue::drain(conversation_id, drain_count, db).await?;
      MessageQueue::finalize_job(conversation_id, new_prev_summary, db).await?;

      enqueue_pending_reviews(conversation_id, batch_messages, db, review_storage).await?;

      // Create episodes for all drained segments in parallel
      let episode_futures: Vec<_> = drain_segments
        .iter()
        .map(|seg| {
          create_episode_from_segment(
            conversation_id,
            &seg.messages,
            &seg.title,
            &seg.summary,
            seg.surprise_level.to_signal(),
            db,
          )
        })
        .collect();

      let created_episodes: Vec<plastmem_core::CreatedEpisode> = try_join_all(episode_futures)
        .await?
        .into_iter()
        .flatten()
        .collect();

      for episode in created_episodes {
        enqueue_semantic_consolidation(conversation_id, episode, db, semantic_storage).await?;
      }

      let _ = last_segment; // last segment stays in queue
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
  episode: plastmem_core::CreatedEpisode,
  db: &DatabaseConnection,
  semantic_storage: &PostgresStorage<SemanticConsolidationJob>,
) -> Result<(), AppError> {
  let is_flashbulb = episode.surprise >= FLASHBULB_SURPRISE_THRESHOLD;
  let unconsolidated_count =
    EpisodicMemory::count_unconsolidated_for_conversation(conversation_id, db).await?;
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
