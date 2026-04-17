use plastmem_event::Event;

#[derive(Debug, Clone)]
pub struct EventSegment {
  events: Vec<Event>,
  reasons: Vec<EventSegmentReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventSegmentReason {
  TopicShift,
  TimeGap,
  IntentShift,
  StructuralCue,
}

impl EventSegment {
  pub fn new(events: Vec<Event>, reasons: Vec<EventSegmentReason>) -> Self {
    Self { events, reasons }
  }

  pub fn events(&self) -> &[Event] {
    &self.events
  }

  pub fn reasons(&self) -> &[EventSegmentReason] {
    &self.reasons
  }
}
