use chrono::TimeDelta;
use plastmem_shared::AppError;
use sea_orm::DatabaseConnection;
use uuid::Uuid;

use super::{MessageQueue, SegmentationAction, SegmentationCheck};
use crate::Message;

/// Minimum number of messages before segmentation is considered.
const MIN_MESSAGES: usize = 3;

/// Maximum buffer size before forcing a split.
const MAX_BUFFER_SIZE: usize = 50;

/// Time gap (in minutes) that triggers a time-based boundary.
const TIME_GAP_MINUTES: i64 = 15;

/// Minimum total character count across all buffered messages.
const MIN_TOTAL_CHARS: usize = 100;

/// Minimum character length of the latest message to trigger boundary evaluation.
const MIN_MESSAGE_LENGTH: usize = 5;

impl MessageQueue {
  /// Check if event segmentation should be triggered.
  /// Returns `Ok(Some(SegmentationCheck))` if segmentation is needed.
  pub async fn check(
    id: Uuid,
    message: &Message,
    db: &DatabaseConnection,
  ) -> Result<Option<SegmentationCheck>, AppError> {
    let messages = Self::get(id, db).await?.messages;

    // === Hard rules (no LLM call) ===

    // Too few messages: never segment.
    if messages.len() < MIN_MESSAGES {
      return Ok(None);
    }

    // Buffer full: force split (drain all messages).
    if messages.len() >= MAX_BUFFER_SIZE {
      return Ok(Some(SegmentationCheck {
        messages,
        action: SegmentationAction::ForceCreate,
      }));
    }

    // Time gap exceeded: boundary (keep last message for next event).
    if messages.last().is_some_and(|last_message| {
      message.timestamp - last_message.timestamp > TimeDelta::minutes(TIME_GAP_MINUTES)
    }) {
      return Ok(Some(SegmentationCheck {
        messages,
        action: SegmentationAction::TimeBoundary,
      }));
    }

    // === Content quality checks ===

    // Total character budget too low — not enough content to segment.
    let total_chars: usize = messages.iter().map(|m| m.content.chars().count()).sum();
    if total_chars < MIN_TOTAL_CHARS {
      return Ok(None);
    }

    // Latest message too short to trigger a boundary evaluation.
    if message.content.chars().count() < MIN_MESSAGE_LENGTH {
      return Ok(None);
    }

    // === Passed rules → needs LLM boundary detection ===
    Ok(Some(SegmentationCheck {
      messages,
      action: SegmentationAction::NeedsBoundaryDetection,
    }))
  }
}
