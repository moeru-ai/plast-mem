mod memory;

mod message_queue;
pub use message_queue::MessageQueue;

mod message;
pub use message::{Message, MessageRole};

mod message_segmenter;
pub use message_segmenter::{SegmentDecision, SegmenterFn, llm_segmenter, rule_segmenter};
