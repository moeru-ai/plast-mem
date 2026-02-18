use apalis::prelude::TaskSink;
use apalis_postgres::PostgresStorage;
use chrono::Utc;
use plastmem_core::{MessageQueue, SegmentationAction, create_episode, detect_boundary};
use plastmem_shared::{AppError, Message};
use sea_orm::DatabaseConnection;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{MemoryReviewJob, SemanticExtractionJob};

// ──────────────────────────────────────────────────
// Job definition
// ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventSegmentationJob {
  pub conversation_id: Uuid,
  pub messages: Vec<Message>,
  pub action: SegmentationAction,
}

// ──────────────────────────────────────────────────
// Job processing
// ──────────────────────────────────────────────────

pub async fn process_event_segmentation(
  job: EventSegmentationJob,
  db: apalis::prelude::Data<DatabaseConnection>,
  review_storage: apalis::prelude::Data<PostgresStorage<MemoryReviewJob>>,
  semantic_storage: apalis::prelude::Data<PostgresStorage<SemanticExtractionJob>>,
) -> Result<(), AppError> {
  let db = &*db;

  // Verify that the job is not stale. The message queue in the database should
  // still contain the messages that this job was created with.
  let current_messages = MessageQueue::get(job.conversation_id, db).await?.messages;
  let job_context_messages = &job.messages[..job.messages.len().saturating_sub(1)];
  if !current_messages.starts_with(job_context_messages) {
    tracing::debug!(
      conversation_id = %job.conversation_id,
      "Skipping stale event segmentation job."
    );
    return Ok(());
  }
  let review_storage = &*review_storage;
  let semantic_storage = &*semantic_storage;
  tracing::debug!(
    conversation_id = %job.conversation_id,
    action = ?job.action,
    messages = job.messages.len(),
    "Processing event segmentation"
  );

  // The last element in job.messages is always the triggering (edge) message.
  // drain_count = len - 1 ensures the edge message stays in the queue for the next event.
  let drain_count = job.messages.len().saturating_sub(1);

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
        messages = job.messages.len(),
        drain_count,
        "{}",
        log_msg
      );
      if drain_count > 0 {
        enqueue_pending_reviews(job.conversation_id, &job.messages, &db, &review_storage).await?;
        if let Some(episode) = create_episode(
          job.conversation_id,
          &job.messages,
          drain_count,
          None,
          0.0,
          &db,
        )
        .await?
        {
          enqueue_semantic_extraction(job.conversation_id, episode, semantic_storage).await?;
        }
      }
    }

    // Needs boundary detection with dual-channel: topic shift + surprise.
    SegmentationAction::NeedsBoundaryDetection => {
      let result = detect_boundary(job.conversation_id, &job.messages, &db).await?;

      if result.is_boundary {
        tracing::info!(
          conversation_id = %job.conversation_id,
          messages = job.messages.len(),
          drain_count,
          surprise = result.surprise_signal,
          "Creating episode (boundary detected)"
        );
        if drain_count > 0 {
          enqueue_pending_reviews(job.conversation_id, &job.messages, &db, &review_storage).await?;
          if let Some(episode) = create_episode(
            job.conversation_id,
            &job.messages,
            drain_count,
            result.latest_embedding,
            result.surprise_signal,
            &db,
          )
          .await?
          {
            enqueue_semantic_extraction(job.conversation_id, episode, semantic_storage).await?;
          }
        }
      } else {
        // No boundary — just process pending reviews, don't drain.
        enqueue_pending_reviews(job.conversation_id, &job.messages, &db, &review_storage).await?;
      }
    }
  }

  Ok(())
}

// ──────────────────────────────────────────────────
// Pending review enqueueing (apalis-dependent)
// ──────────────────────────────────────────────────

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

// ──────────────────────────────────────────────────
// Semantic extraction enqueueing
// ──────────────────────────────────────────────────

/// Enqueue a SemanticExtractionJob after an episode is created.
async fn enqueue_semantic_extraction(
  conversation_id: Uuid,
  episode: plastmem_core::CreatedEpisode,
  semantic_storage: &PostgresStorage<SemanticExtractionJob>,
) -> Result<(), AppError> {
  let job = SemanticExtractionJob {
    episode_id: episode.id,
    conversation_id,
    summary: episode.summary,
    messages: episode.messages,
    surprise: episode.surprise,
  };
  let mut storage = semantic_storage.clone();
  storage.push(job).await?;
  tracing::debug!(
    episode_id = %episode.id,
    conversation_id = %conversation_id,
    "Enqueued semantic extraction job"
  );
  Ok(())
}
