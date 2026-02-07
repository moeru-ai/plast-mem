use crate::{Message, MessageRole};

pub type SegmenterFn = dyn Fn(&[Message], &Message) -> bool + Send + Sync;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentDecision {
  Split,
  NoSplit,
  CallLlm,
}

pub fn llm_segmenter(_recent: &[Message], _incoming: &Message) -> bool {
  false
}

pub fn rule_segmenter(recent: &[Message], incoming: &Message) -> SegmentDecision {
  let last = match recent.last() {
    Some(message) => message,
    None => return SegmentDecision::NoSplit,
  };

  let interval = incoming.timestamp - last.timestamp;

  match incoming.role {
    MessageRole::User => {
      if interval > chrono::Duration::minutes(30) {
        return SegmentDecision::Split;
      }
      if incoming.content.len() > 99 {
        return SegmentDecision::Split;
      }
      if incoming.content.len() < 5 {
        return SegmentDecision::NoSplit;
      }
      SegmentDecision::CallLlm
    }
    MessageRole::Assistant => {
      if interval > chrono::Duration::minutes(10) {
        SegmentDecision::Split
      } else {
        SegmentDecision::NoSplit
      }
    }
  }
}
