use plastmem_shared::Message;

#[derive(Debug, Clone)]
pub struct EventSegment {
  messages: Vec<Message>,
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
  pub fn new(messages: Vec<Message>, reasons: Vec<EventSegmentReason>) -> Self {
    Self { messages, reasons }
  }

  pub fn messages(&self) -> &[Message] {
    &self.messages
  }

  pub fn reasons(&self) -> &[EventSegmentReason] {
    &self.reasons
  }
}
