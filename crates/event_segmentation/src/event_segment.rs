use plastmem_event::Event;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::AsRefStr;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventSegment {
  pub events: Vec<Event>,
  pub reasons: Vec<EventSegmentReason>,
  pub score: f32,
  pub boundary_before_confidence: f32,
  pub boundary_after_confidence: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, AsRefStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum EventSegmentReason {
  TopicShift,
  TimeGap,
  HardTimeGap,
  IntentShift,
  StructuralCue,
}

impl EventSegmentReason {
  pub fn is_time_gap(self) -> bool {
    matches!(self, Self::TimeGap | Self::HardTimeGap)
  }
}

impl EventSegment {
  pub fn new(events: Vec<Event>, reasons: Vec<EventSegmentReason>) -> Self {
    Self {
      events,
      reasons,
      score: 1.0,
      boundary_before_confidence: 1.0,
      boundary_after_confidence: 1.0,
    }
  }

  pub fn with_metadata(
    events: Vec<Event>,
    reasons: Vec<EventSegmentReason>,
    score: f32,
    boundary_before_confidence: f32,
    boundary_after_confidence: f32,
  ) -> Self {
    Self {
      events,
      reasons,
      score,
      boundary_before_confidence,
      boundary_after_confidence,
    }
  }

  pub fn prepend_reason_if_missing(mut self, reason: Option<EventSegmentReason>) -> Self {
    if let Some(reason) = reason
      && !self.reasons.contains(&reason)
    {
      self.reasons.insert(0, reason);
    }

    self
  }

  pub fn replace_reason(mut self, reason: EventSegmentReason) -> Self {
    self.reasons = vec![reason];
    self
  }

  pub fn extend_events(&mut self, events: impl IntoIterator<Item = Event>) {
    self.events.extend(events);
  }
}
