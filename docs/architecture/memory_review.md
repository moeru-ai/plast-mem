# Memory Review

## Overview

Memory review evaluates retrieved memories' relevance to the conversation context and updates their FSRS parameters. An LLM reviewer assigns Again/Hard/Good/Easy ratings based on actual memory usage.

**Core principle**: review ≠ retrieval. Retrieval is search; review is post-hoc evaluation. Only review updates FSRS parameters.

## Flow

```
retrieve_memory(query, conversation_id)
  → Normal retrieval returns results (no FSRS update)
  → Records pending review: { query, memory_ids }

Conversation continues...

When segmentation triggers:
  → Take pending_reviews (atomically read + clear)
  → Enqueue MemoryReviewJob if any exist
  → Execute segmentation

MemoryReviewJob (async worker):
  → Aggregate: deduplicate memories, collect matched queries
  → LLM evaluates relevance in conversation context
  → Update FSRS parameters based on rating
```

## Data Structures

### [PendingReview](../../crates/core/src/message_queue.rs)

Stored in `message_queue.pending_reviews` as JSON array. `NULL` means no pending reviews.

### [MemoryReviewJob](../../crates/worker/src/jobs/memory_review.rs)

## Key Components

### 1. Recording Pending Reviews

After retrieval, [`fetch_memory()`](../../crates/server/src/api/retrieve_memory.rs) appends the retrieved memory IDs to the queue via [`MessageQueue::add_pending_review()`](../../crates/core/src/message_queue.rs).

### 2. Enqueueing Review Job

During event segmentation, [`enqueue_pending_reviews()`](../../crates/worker/src/jobs/event_segmentation.rs) atomically takes pending reviews and enqueues a `MemoryReviewJob`. Called on all segmentation paths.

### 3. Processing Reviews

[`process_memory_review()`](../../crates/worker/src/jobs/memory_review.rs):

1. **Aggregate**: Deduplicate by `memory_id`, collect matched queries
2. **Fetch**: Load memory content from DB, skip if stale or same-day
3. **LLM Review**: Call [`generate_object()`](../../crates/ai/src/lib.rs) with structured output
4. **Update**: Apply FSRS state transition based on rating

### 4. Stale Skip

Prevents race conditions: if `job.reviewed_at <= memory.last_reviewed_at`, skip the update. Same-day reviews are also skipped (at least 1 day between reviews).

## LLM Review Prompt

**System prompt** (see [`REVIEW_SYSTEM_PROMPT`](../../crates/worker/src/jobs/memory_review.rs)):

```
You are a memory relevance reviewer. Evaluate how relevant each retrieved memory was to the conversation context.

For each memory, assign a rating:
- "again": Memory was not used in the conversation at all. It is noise.
- "hard": Memory is tangentially related but required significant inference to connect.
- "good": Memory is directly relevant and visibly influenced the conversation.
- "easy": Memory is a core pillar of the conversation. The conversation could not have proceeded meaningfully without it.
```

**Output format**:

```rust
#[derive(Debug, Deserialize, JsonSchema)]
struct MemoryReviewOutput {
    pub ratings: Vec<MemoryRatingOutput>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct MemoryRatingOutput {
    pub memory_id: String,
    pub rating: String, // "again" | "hard" | "good" | "easy"
}
```

## Rating Definitions

| Rating | Judgment Criteria | FSRS Effect |
|--------|------------------|-------------|
| Again | Unrelated, not used | Stability drops significantly |
| Hard | Related but requires inference | Stability unchanged |
| Good | Directly relevant, visibly used | Stability increases moderately |
| Easy | Core pillar of conversation | Stability increases substantially, difficulty decreases |

## Related Files

- `crates/core/src/message_queue.rs` - `PendingReview` struct, `add_pending_review()`, `take_pending_reviews()`
- `crates/server/src/api/retrieve_memory.rs` - Records pending reviews
- `crates/worker/src/jobs/event_segmentation.rs` - Enqueues review job
- `crates/worker/src/jobs/memory_review.rs` - Review job implementation
