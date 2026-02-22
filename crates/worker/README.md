# plastmem_worker

Background job worker for Plast Mem.

## Overview

Runs two background job processors:

1. **Event Segmentation** - Processes message queues, detects event boundaries, creates episodic memories
2. **Memory Review** - Evaluates retrieved memories and updates FSRS parameters

Uses [Apalis](https://github.com/apalis-rs/apalis) for job queue management with PostgreSQL storage.

## Job Types

### EventSegmentationJob

Triggered when new messages are added to a conversation:

```rust
pub struct EventSegmentationJob {
    pub conversation_id: Uuid,
    pub trigger: Message,
    pub action: SegmentationAction,
}
```

Processing flow:
1. Fetch current `MessageQueue` from DB; find `trigger` message's position
2. If trigger not found, skip (stale job)
3. Process messages up to and including trigger; drain all but trigger (edge message stays for next event)
4. Run boundary detection or force-create episode
5. Enqueue pending reviews for `MemoryReviewJob`

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

## Usage

Start the worker:

```rust
use plastmem_worker::worker;

worker(db, segmentation_storage, review_storage).await?;
```

This runs indefinitely until SIGINT (Ctrl+C) is received.

## Worker Configuration

Each worker has:
- **Name**: "event-segmentation" or "memory-review"
- **Tracing**: Enabled via `enable_tracing()`
- **Shutdown timeout**: 5 seconds

## Error Handling

Jobs use `WorkerError` as a boundary type to satisfy Apalis constraints.
Internal errors are `AppError`, converted at the job boundary.

## Module Structure

- `jobs/mod.rs` - Job definitions and error types
- `jobs/event_segmentation.rs` - Segmentation job implementation
- `jobs/memory_review.rs` - Review job implementation
- `lib.rs` - Worker registration and monitor setup
