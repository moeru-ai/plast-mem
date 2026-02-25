# plastmem_worker

Background job worker for Plast Mem.

## Overview

Runs three background job processors:

1. **Event Segmentation** - Batch-segments message queues and creates episodic memories
2. **Memory Review** - Evaluates retrieved memories and updates FSRS parameters
3. **Semantic Consolidation** - Extracts long-term facts from episodic memories

Uses [Apalis](https://github.com/apalis-rs/apalis) for job queue management with PostgreSQL storage.

## Job Types

### EventSegmentationJob

Triggered when `MessageQueue::push()` returns a `SegmentationCheck`:

```rust
pub struct EventSegmentationJob {
    pub conversation_id: Uuid,
    /// Exact message count at push time (fence boundary).
    pub fence_count: i32,
}
```

Processing flow:

1. Fetch queue; validate fence (skip if stale)
2. Run `batch_segment(messages[0..fence_count])` — single LLM call
3. **Drain + finalize first** (crash-safe: loss preferred over duplicate)
4. Create episodes for drained segments in parallel (`try_join_all`)
5. Enqueue `SemanticConsolidationJob` per episode if threshold reached

Window doubling: if LLM returns 1 segment and window not yet doubled → double window, clear fence, wait for more messages.

### MemoryReviewJob

Triggered after retrieval to evaluate memory relevance:

```rust
pub struct MemoryReviewJob {
    pub pending_reviews: Vec<PendingReview>,
    pub context_messages: Vec<Message>,
    pub reviewed_at: DateTime<Utc>,
}
```

Processing flow:

1. Aggregate pending reviews (deduplicate memory IDs)
2. Call LLM to evaluate relevance (Again/Hard/Good/Easy)
3. Update FSRS parameters based on rating

### SemanticConsolidationJob

Triggered after episode creation when unconsolidated episode count ≥ threshold (3) or surprise ≥ 0.85:

```rust
pub struct SemanticConsolidationJob {
    pub conversation_id: Uuid,
    pub force: bool,  // true for flashbulb (surprise ≥ 0.85)
}
```

Processing flow:

1. Fetch unconsolidated episodes for the conversation
2. Check threshold (skip if below, unless `force=true`)
3. Load related existing facts as context
4. Single LLM call → fact actions (new/reinforce/update/invalidate)
5. Embed new facts, apply actions in a transaction
6. Mark episodes as consolidated

## Usage

Start the worker:

```rust
use plastmem_worker::worker;

worker(db, segmentation_storage, review_storage, semantic_storage).await?;
```

This runs indefinitely until SIGINT (Ctrl+C) is received.

## Worker Configuration

Each worker has:

- **Name**: "event-segmentation", "memory-review", or "semantic-consolidation"
- **Tracing**: Enabled via `enable_tracing()`
- **Shutdown timeout**: 5 seconds

## Error Handling

Jobs use `WorkerError` as a boundary type to satisfy Apalis constraints.
Internal errors are `AppError`, converted at the job boundary.

## Module Structure

- `jobs/mod.rs` - Job definitions and error types
- `jobs/event_segmentation.rs` - Segmentation job implementation
- `jobs/memory_review.rs` - Review job implementation
- `jobs/semantic_consolidation.rs` - Consolidation job implementation
- `lib.rs` - Worker registration and monitor setup
