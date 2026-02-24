# plastmem_core

Core domain logic for Plast Mem.

## Overview

This crate implements the central business logic:

- **Episodic Memory** - Storage and retrieval of conversation segments
- **Semantic Memory** - Long-term fact storage and consolidation
- **Message Queue** - Buffering, trigger detection, and batch segmentation
- **Episode Creation** - Embedding + FSRS initialization from batch segment output

## Key Types

### EpisodicMemory

The main memory type representing a conversation segment:

```rust
pub struct EpisodicMemory {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub messages: Vec<Message>,     // Original conversation
    pub title: String,              // From batch segmentation LLM
    pub summary: String,            // From batch segmentation LLM
    pub stability: f32,             // FSRS stability
    pub difficulty: f32,            // FSRS difficulty
    pub surprise: f32,              // 0.0-1.0 significance score
    pub start_at: DateTime<Utc>,
    pub end_at: DateTime<Utc>,
    // ... timestamps
}
```

### MessageQueue

Per-conversation message buffer:

```rust
// Push a message; returns Some(SegmentationCheck) if a job should be enqueued
let check = MessageQueue::push(conversation_id, message, db).await?;

// Drain the first N messages after a job completes
MessageQueue::drain(conversation_id, count, db).await?;
```

### PendingReview

Tracks memories retrieved for later FSRS review:

```rust
pub struct PendingReview {
    pub query: String,              // The search query
    pub memory_ids: Vec<Uuid>,      // Retrieved memory IDs
}
```

### BatchSegment / SurpriseLevel

Output types from `batch_segment()`:

```rust
pub struct BatchSegment {
    pub messages: Vec<Message>,     // Sliced from queue
    pub title: String,
    pub summary: String,
    pub surprise_level: SurpriseLevel,  // Low / High / ExtremelyHigh
}
```

## Key Functions

### Retrieval

```rust
use plastmem_core::{EpisodicMemory, DetailLevel};

let results = EpisodicMemory::retrieve(
    "user query",
    5,                              // limit
    Some(conversation_id),         // scope (None for global)
    db,
).await?;
```

### Batch Segmentation

```rust
use plastmem_core::batch_segment;

// Single LLM call; returns segments with title, summary, surprise_level
let segments = batch_segment(&messages, prev_episode_summary).await?;
```

### Episode Creation

```rust
use plastmem_core::create_episode_from_segment;

let created = create_episode_from_segment(
    conversation_id,
    &segment.messages,
    &segment.title,
    &segment.summary,
    segment.surprise_level.to_signal(),
    db,
).await?;
```

## Modules

- `memory/episodic/mod.rs` - `EpisodicMemory` struct and hybrid retrieval
- `memory/episodic/creation.rs` - Episode creation and FSRS initialization
- `memory/semantic/mod.rs` - Semantic memory retrieval
- `memory/semantic/consolidation.rs` - CLS consolidation pipeline
- `memory/retrieval.rs` - Shared markdown formatting (`format_tool_result`, `DetailLevel`)
- `message_queue/mod.rs` - `MessageQueue`, `PendingReview`, push/drain
- `message_queue/check.rs` - Trigger check, fence acquisition, `SegmentationCheck`
- `message_queue/segmentation.rs` - `batch_segment`, `BatchSegment`, `SurpriseLevel`
- `message_queue/state.rs` - Fence state management, pending reviews

## Architecture Notes

- Core logic is pure domain code — no HTTP or job queue specifics
- Database operations use Sea-ORM
- LLM calls go through `plastmem_ai`
