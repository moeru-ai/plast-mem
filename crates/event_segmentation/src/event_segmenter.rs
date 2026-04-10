use chrono::TimeDelta;
use plastmem_shared::{AppError, Message};

use crate::{EventSegment, EventSegmentReason};

pub struct EventSegmenter {}

impl EventSegmenter {
  const TIME_GAP_THRESHOLD: TimeDelta = TimeDelta::minutes(30);

  // Perform segmented processing on messages with intervals exceeding 30 minutes.
  fn segment_by_time_gap(messages: &[Message]) -> Result<Vec<EventSegment>, AppError> {
    if messages.is_empty() {
      return Ok(Vec::new());
    }

    let mut segments = Vec::new();

    let mut curr_messages = vec![messages[0].clone()];
    let mut curr_reasons = Vec::new();
    let mut prev = &messages[0];

    for curr in messages.iter().skip(1) {
      let gap = curr.timestamp.signed_duration_since(prev.timestamp);

      if gap > Self::TIME_GAP_THRESHOLD {
        segments.push(EventSegment::new(
          std::mem::take(&mut curr_messages),
          std::mem::take(&mut curr_reasons),
        ));
        curr_messages.push(curr.clone());
        curr_reasons.push(EventSegmentReason::TimeGap);
      } else {
        curr_messages.push(curr.clone());
      }

      prev = curr;
    }

    segments.push(EventSegment::new(curr_messages, curr_reasons));
    Ok(segments)
  }

  pub fn segment(messages: &[Message]) -> Result<Vec<EventSegment>, AppError> {
    Self::segment_by_time_gap(messages)
  }
}
