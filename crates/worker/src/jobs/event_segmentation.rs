use apalis::prelude::TaskSink;
use apalis_postgres::PostgresStorage;
use chrono::Utc;
use plastmem_core::{Message, MessageQueue, SegmentationAction, create_episode, detect_boundary};
use plastmem_shared::AppError;
use sea_orm::DatabaseConnection;
use serde::{Deserialize, Serialize};
use tracing::info;
use uuid::Uuid;

use super::MemoryReviewJob;

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
) -> Result<(), AppError> {
  let db = &*db;
  let review_storage = &*review_storage;
  info!(
    conversation_id = %job.conversation_id,
    action = ?job.action,
    messages = job.messages.len(),
    "Processing event segmentation"
  );

  // The last element in job.messages is always the triggering (edge) message.
  // drain_count = len - 1 ensures the edge message stays in the queue for the next event.
  let drain_count = job.messages.len().saturating_sub(1);

  match job.action {
    // Force-create: skip boundary detection, go straight to episode generation.
    SegmentationAction::ForceCreate => {
      info!(
        conversation_id = %job.conversation_id,
        messages = job.messages.len(),
        drain_count,
        "Force-creating episode (buffer full)"
      );
      if drain_count > 0 {
        enqueue_pending_reviews(job.conversation_id, &job.messages, &db, &review_storage).await?;
        create_episode(
          job.conversation_id,
          &job.messages,
          drain_count,
          None,
          0.0,
          &db,
        )
        .await?;
      }
    }

    // Time boundary: skip boundary detection, create episode.
    SegmentationAction::TimeBoundary => {
      info!(
        conversation_id = %job.conversation_id,
        messages = job.messages.len(),
        drain_count,
        "Creating episode (time boundary)"
      );
      if drain_count > 0 {
        enqueue_pending_reviews(job.conversation_id, &job.messages, &db, &review_storage).await?;
        create_episode(
          job.conversation_id,
          &job.messages,
          drain_count,
          None,
          0.0,
          &db,
        )
        .await?;
      }
    }

    // Needs boundary detection with dual-channel: topic shift + surprise.
    SegmentationAction::NeedsBoundaryDetection => {
      let result = detect_boundary(job.conversation_id, &job.messages, &db).await?;

      if result.is_boundary {
        info!(
          conversation_id = %job.conversation_id,
          messages = job.messages.len(),
          drain_count,
          surprise = result.surprise_signal,
          "Creating episode (boundary detected)"
        );
        if drain_count > 0 {
          enqueue_pending_reviews(job.conversation_id, &job.messages, &db, &review_storage).await?;
          create_episode(
            job.conversation_id,
            &job.messages,
            drain_count,
            result.latest_embedding,
            result.surprise_signal,
            &db,
          )
          .await?;
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
