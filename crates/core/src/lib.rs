mod memory;
mod segmentation;
pub use memory::EpisodicMemory;
pub use memory::SemanticMemory;
pub use memory::{DetailLevel, format_tool_result};

mod message_queue;
pub use message_queue::{MessageQueue, PendingReview};
pub use segmentation::{
  ADD_BACKPRESSURE_LIMIT, ConversationMessageRecord, IngestResult, SEGMENTATION_GAP_TRIGGER_HOURS,
  SEGMENTATION_IN_PROGRESS_TTL_MINUTES, SEGMENTATION_WINDOW_BASE, SEGMENTATION_WINDOW_MAX,
  SegmentationBoundaryContext, SegmentationProcessingStatus, SegmentationState, append_messages,
  clear_stale_in_progress, ensure_segmentation_state, get_processing_status,
  get_segmentation_state, list_messages,
};
