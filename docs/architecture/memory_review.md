# Memory Review

Memory review is the FSRS update path for episodic memories.

It is decoupled from retrieval and from segmentation policy.

## Flow

```text
retrieve_memory / retrieve_memory_raw
  -> add_pending_review_item(conversation_id, memory_ids, query)

Later, after segmentation commit:
  -> take_pending_review_items(conversation_id)
  -> enqueue MemoryReviewJob

MemoryReviewJob
  -> aggregate memory ids and matched queries
  -> load episodic records
  -> skip stale / same-day reviews
  -> ask review LLM for ratings
  -> apply FSRS next_states update
```

## Storage

Pending review items now live in the dedicated `pending_review_queue` table.

They no longer live inside a `message_queue` row.

## Rating model

The LLM emits one rating per memory:

- `again`
- `hard`
- `good`
- `easy`

The worker maps that to FSRS next states using the current `stability`,
`difficulty`, and days since the last review.

## Current guards

Before applying an update, the worker skips a memory when:

- the record no longer exists
- `job.reviewed_at <= last_reviewed_at`
- fewer than 1 day has elapsed since `last_reviewed_at`

## Code

- `crates/server/src/api/retrieve_memory.rs`
- `crates/core/src/pending_review_queue.rs`
- `crates/worker/src/jobs/event_segmentation.rs`
- `crates/worker/src/jobs/memory_review.rs`
