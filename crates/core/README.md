# plastmem_core

Core domain logic for Plast Mem.

## Overview

This crate implements the central business logic:

- **Episodic Memory** - Storage and retrieval of conversation segments
- **Semantic Memory** - Long-term fact storage and consolidation
- **Message Queue** - Buffering, trigger detection, and batch segmentation

## Key Types

### [EpisodicMemory](src/memory/episodic.rs)

The main memory type representing a conversation segment with FSRS parameters for spaced repetition scheduling.

### [SemanticMemory](src/memory/semantic.rs)

A long-term fact extracted from episodic memories. Uses 8 categories: `identity`, `preference`, `interest`, `personality`, `relationship`, `experience`, `goal`, `guideline`.

### [MessageQueue](src/message_queue.rs)

Per-conversation message buffer with segmentation trigger logic.

Key operations:
```rust
// Push a message; returns Some(SegmentationCheck) if a job should be enqueued
let check = MessageQueue::push(conversation_id, message, db).await?;

// Drain the first N messages after a job completes
MessageQueue::drain(conversation_id, count, db).await?;
```

### [PendingReview](src/message_queue.rs)

Tracks memories retrieved for later FSRS review.

## Key Functions

### Episodic Retrieval

```rust
let results = EpisodicMemory::retrieve(query, limit, conversation_id, db).await?;
```

### Semantic Retrieval

```rust
// Full hybrid search (BM25 on search_text + vector), optional category filter
let results = SemanticMemory::retrieve(query, limit, conversation_id, db, category).await?;

// Pre-embedded variant (used inside consolidation to avoid re-embedding)
let results = SemanticMemory::retrieve_by_embedding(
    query, embedding, limit, conversation_id, db, category
).await?;
```

## Modules

- `memory/episodic.rs` - `EpisodicMemory` struct, hybrid BM25 + vector retrieval with FSRS re-ranking
- `memory/semantic.rs` - `SemanticMemory` struct, hybrid BM25 + vector retrieval (no FSRS)
- `memory/retrieval.rs` - Shared markdown formatting (`format_tool_result`, `DetailLevel`)
- `message_queue.rs` - `MessageQueue`, `PendingReview`, `SegmentationCheck`, push/drain/get

## Architecture Notes

- Core logic is pure domain code — no HTTP or job queue specifics
- Database operations use Sea-ORM
- LLM calls go through `plastmem_ai`
