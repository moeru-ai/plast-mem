use chrono::TimeDelta;
use plastmem_shared::AppError;

use crate::{Event, EventSegment, EventSegmentReason};

pub struct EventSegmenter {}

impl EventSegmenter {
  const TIME_GAP_THRESHOLD: TimeDelta = TimeDelta::minutes(30);

  // Perform segmented processing on events with intervals exceeding 30 minutes.
  fn segment_by_time_gap(events: &[Event]) -> Result<Vec<EventSegment>, AppError> {
    if events.is_empty() {
      return Ok(Vec::new());
    }

    let mut segments = Vec::new();

    let mut curr_events = vec![events[0].clone()];
    let mut curr_reasons = Vec::new();
    let mut prev = &events[0];

    for curr in events.iter().skip(1) {
      let gap = curr.timestamp().signed_duration_since(prev.timestamp());

      if gap > Self::TIME_GAP_THRESHOLD {
        segments.push(EventSegment::new(
          std::mem::take(&mut curr_events),
          std::mem::take(&mut curr_reasons),
        ));
        curr_events.push(curr.clone());
        curr_reasons.push(EventSegmentReason::TimeGap);
      } else {
        curr_events.push(curr.clone());
      }

      prev = curr;
    }

    segments.push(EventSegment::new(curr_events, curr_reasons));
    Ok(segments)
  }

  pub fn segment(events: &[Event]) -> Result<Vec<EventSegment>, AppError> {
    Self::segment_by_time_gap(events)
  }
}
