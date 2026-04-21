use plastmem_event::Event;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct EventSegment {
  pub events: Vec<Event>,
  pub reasons: Vec<EventSegmentReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
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
    Self { events, reasons }
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
