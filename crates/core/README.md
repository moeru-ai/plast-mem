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
    pub title: String,
    pub summary: String,
    pub stability: f32,             // FSRS stability
    pub difficulty: f32,            // FSRS difficulty
    pub surprise: f32,              // 0.0-1.0 significance score
    pub start_at: DateTime<Utc>,
    pub end_at: DateTime<Utc>,
    // ... timestamps
}
```

### SemanticMemory

A long-term fact extracted from episodic memories:

```rust
pub struct SemanticMemory {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub fact: String,               // Natural language sentence
    pub source_episodic_ids: Vec<Uuid>,
    pub valid_at: DateTime<Utc>,
    pub invalid_at: Option<DateTime<Utc>>,
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

## Key Functions

### Episodic Retrieval

```rust
let results = EpisodicMemory::retrieve(query, limit, conversation_id, db).await?;
```

### Semantic Retrieval

```rust
let results = SemanticMemory::retrieve(query, limit, conversation_id, db).await?;
```

## Modules

- `memory/episodic.rs` - `EpisodicMemory` struct, hybrid BM25 + vector retrieval with FSRS re-ranking
- `memory/semantic.rs` - `SemanticMemory` struct, hybrid BM25 + vector retrieval
- `memory/retrieval.rs` - Shared markdown formatting (`format_tool_result`, `DetailLevel`)
- `message_queue.rs` - `MessageQueue`, `PendingReview`, `SegmentationCheck`, push/drain/get

## Architecture Notes

- Core logic is pure domain code — no HTTP or job queue specifics
- Database operations use Sea-ORM
- LLM calls go through `plastmem_ai`
