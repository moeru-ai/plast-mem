use chrono::{TimeDelta, Utc};
use plastmem_shared::AppError;
use sea_orm::DatabaseConnection;
use uuid::Uuid;

use super::MessageQueue;

// ──────────────────────────────────────────────────
// Trigger constants
// ──────────────────────────────────────────────────

/// Minimum number of messages before segmentation is considered.
const MIN_MESSAGES: usize = 5;

/// Base window size (number of messages that triggers a batch segmentation job).
const WINDOW_BASE: usize = 20;

/// Maximum window size after doubling (2× base).
const WINDOW_MAX: usize = 40;

/// TTL in minutes for stale fence recovery.
const FENCE_TTL_MINUTES: i64 = 120;

/// Soft time trigger: if the oldest message in the queue is older than this, trigger segmentation.
const SOFT_TIME_TRIGGER_HOURS: i64 = 2;

// ──────────────────────────────────────────────────
// Result type
// ──────────────────────────────────────────────────

/// Result of checking if event segmentation is needed.
#[derive(Debug, Clone)]
pub struct SegmentationCheck {
  /// Number of messages captured in the fence (job processes messages[0..fence_count]).
  pub fence_count: i32,
}

// ──────────────────────────────────────────────────
// Trigger check
// ──────────────────────────────────────────────────

impl MessageQueue {
  /// Check if batch segmentation should be triggered.
  ///
  /// `trigger_count` is the exact message count returned by the push operation (via RETURNING).
  /// It represents the fence boundary: only messages[0..trigger_count] belong to this batch,
  /// even if more messages arrive concurrently before the fence is acquired.
  ///
  /// Returns `Ok(Some(SegmentationCheck))` if a job should be created.
  pub async fn check(
    id: Uuid,
    trigger_count: i32,
    db: &DatabaseConnection,
  ) -> Result<Option<SegmentationCheck>, AppError> {
    let model = MessageQueue::get_or_create_model(id, db).await?;

    // === Fence check ===
    if model.in_progress_fence.is_some() {
      // Attempt to clear if stale; if not stale, another job is active.
      let cleared = MessageQueue::clear_stale_fence(id, FENCE_TTL_MINUTES, db).await?;
      if !cleared {
        tracing::debug!(
          conversation_id = %id,
          "Segmentation skipped: job in progress"
        );
        return Ok(None);
      }
      // Stale fence was cleared; fall through to trigger evaluation.
    }

    // === Minimum message floor ===
    let trigger_count_usize = trigger_count as usize;
    if trigger_count_usize < MIN_MESSAGES {
      return Ok(None);
    }

    // === Determine current window size ===
    let current_window = if model.window_doubled {
      WINDOW_MAX
    } else {
      WINDOW_BASE
    };

    // === Trigger conditions (OR) ===
    // Count trigger uses the exact push-time count, not a re-read (avoids TOCTOU).
    let count_trigger = trigger_count_usize >= current_window;
    // Time trigger still needs the oldest message's timestamp from DB.
    let messages: Vec<plastmem_shared::Message> = serde_json::from_value(model.messages)?;
    let time_trigger = messages.first().is_some_and(|first| {
      Utc::now() - first.timestamp > TimeDelta::hours(SOFT_TIME_TRIGGER_HOURS)
    });

    if !count_trigger && !time_trigger {
      return Ok(None);
    }

    // === Atomically acquire fence at the exact trigger boundary ===
    // Pass trigger_count explicitly so the fence is set to THIS push's position,
    // not jsonb_array_length(messages) which may have grown by this point.
    if !MessageQueue::try_set_fence(id, trigger_count, db).await? {
      // Another concurrent request won the race
      return Ok(None);
    }

    tracing::debug!(
      conversation_id = %id,
      trigger_count,
      count_trigger,
      time_trigger,
      window_doubled = model.window_doubled,
      "Segmentation triggered"
    );

    Ok(Some(SegmentationCheck { fence_count: trigger_count }))
  }
}
