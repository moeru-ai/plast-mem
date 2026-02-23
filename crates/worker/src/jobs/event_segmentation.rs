use apalis::prelude::{Data, TaskSink};
use apalis_postgres::PostgresStorage;
use chrono::Utc;
use plastmem_core::{
  CONSOLIDATION_EPISODE_THRESHOLD, EpisodicMemory, FLASHBULB_SURPRISE_THRESHOLD, MessageQueue,
  SegmentationAction, create_episode, detect_boundary,
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
  pub trigger: Message,
  pub action: SegmentationAction,
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

  // Fetch current queue state. The job snapshot may be stale if messages arrived
  // while this job was queued. Find the trigger message's position in the current queue.
  let current_messages = MessageQueue::get(job.conversation_id, db).await?.messages;
  let Some(trigger_idx) = current_messages.iter().position(|m| m == &job.trigger) else {
    tracing::debug!(
      conversation_id = %job.conversation_id,
      "Skipping stale event segmentation job."
    );
    return Ok(());
  };

  // Only process messages up to and including the trigger message.
  // drain_count = trigger_idx: drain [0, trigger_idx-1], keep trigger as next event's start.
  let messages = &current_messages[..=trigger_idx];
  let drain_count = trigger_idx;
  let review_storage = &*review_storage;
  let semantic_storage = &*semantic_storage;
  tracing::debug!(
    conversation_id = %job.conversation_id,
    action = ?job.action,
    messages = messages.len(),
    "Processing event segmentation"
  );

  match job.action {
    // Force-create and Time-boundary both skip boundary detection, go straight to episode generation.
    SegmentationAction::ForceCreate | SegmentationAction::TimeBoundary => {
      let log_msg = if matches!(job.action, SegmentationAction::ForceCreate) {
        "Force-creating episode (buffer full)"
      } else {
        "Creating episode (time boundary)"
      };
      tracing::info!(
        conversation_id = %job.conversation_id,
        messages = messages.len(),
        drain_count,
        "{}",
        log_msg
      );
      if drain_count > 0 {
        enqueue_pending_reviews(job.conversation_id, messages, db, review_storage).await?;
        if let Some(episode) = create_episode(
          job.conversation_id,
          messages,
          drain_count,
          None,
          0.0,
          db,
        )
        .await?
        {
          enqueue_semantic_consolidation(job.conversation_id, episode, db, semantic_storage)
            .await?;
        }
      }
    }

    // Needs boundary detection with dual-channel: topic shift + surprise.
    SegmentationAction::NeedsBoundaryDetection => {
      let result = detect_boundary(job.conversation_id, messages, db).await?;

      if result.is_boundary {
        tracing::info!(
          conversation_id = %job.conversation_id,
          messages = messages.len(),
          drain_count,
          surprise = result.surprise_signal,
          "Creating episode (boundary detected)"
        );
        if drain_count > 0 {
          enqueue_pending_reviews(job.conversation_id, messages, db, review_storage).await?;
          if let Some(episode) = create_episode(
            job.conversation_id,
            messages,
            drain_count,
            result.latest_embedding,
            result.surprise_signal,
            db,
          )
          .await?
          {
            enqueue_semantic_consolidation(job.conversation_id, episode, db, semantic_storage)
              .await?;
          }
        }
      } else {
        // No boundary — just process pending reviews, don't drain.
        enqueue_pending_reviews(job.conversation_id, messages, db, review_storage).await?;
      }
    }
  }

  Ok(())
}

// ──────────────────────────────────────────────────
// Pending review enqueueing (apalis-dependent)
// ──────────────────────────────────────────────────

/// Take pending reviews from the queue and enqueue a `MemoryReviewJob` if any exist.
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
    let mut review_job_storage = review_storage.clone();
    review_job_storage.push(review_job).await?;
  }
  Ok(())
}

/// Enqueue a `SemanticConsolidationJob` if threshold is met or it's a flashbulb memory.
async fn enqueue_semantic_consolidation(
  conversation_id: Uuid,
  episode: plastmem_core::CreatedEpisode,
  db: &DatabaseConnection,
  semantic_storage: &PostgresStorage<SemanticConsolidationJob>,
) -> Result<(), AppError> {
  // Check if we should trigger consolidation
  // 1. Flashbulb memory (high surprise) -> immediate force consolidation
  // 2. Threshold reached (>= 3 unconsolidated episodes) -> standard consolidation

  let is_flashbulb = episode.surprise >= FLASHBULB_SURPRISE_THRESHOLD;
  let unconsolidated_count =
    EpisodicMemory::count_unconsolidated_for_conversation(conversation_id, db).await?;
  let threshold_reached = unconsolidated_count >= CONSOLIDATION_EPISODE_THRESHOLD;

  if is_flashbulb || threshold_reached {
    let job = SemanticConsolidationJob {
      conversation_id,
      force: is_flashbulb,
    };
    let mut semantic_job_storage = semantic_storage.clone();
    semantic_job_storage.push(job).await?;
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
