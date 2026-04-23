use plastmem_event::Event;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::AsRefStr;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventSegment {
  pub events: Vec<Event>,
  pub reason: EventSegmentReason,
  pub score: f32,
  pub boundary_before_confidence: f32,
  pub boundary_after_confidence: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, AsRefStr)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum EventSegmentReason {
  InitialSegment,
  TopicShift,
  TimeGap,
  HardTimeGap,
  IntentShift,
  ActivityShift,
  StructuralCue,
}

impl EventSegment {
  pub fn new(events: Vec<Event>, reason: EventSegmentReason) -> Self {
    Self {
      events,
      reason,
      score: 1.0,
      boundary_before_confidence: 1.0,
      boundary_after_confidence: 1.0,
    }
  }

  pub fn with_metadata(
    events: Vec<Event>,
    reason: EventSegmentReason,
    score: f32,
    boundary_before_confidence: f32,
    boundary_after_confidence: f32,
  ) -> Self {
    Self {
      events,
      reason,
      score,
      boundary_before_confidence,
      boundary_after_confidence,
    }
  }

  pub fn extend_events(&mut self, events: impl IntoIterator<Item = Event>) {
    self.events.extend(events);
  }
}
