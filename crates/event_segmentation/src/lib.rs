mod legacy;
pub use legacy::*;

mod event_segment;
pub use event_segment::{EventSegment, EventSegmentReason};

mod event_segmenter;
pub use event_segmenter::EventSegmenter;
