# Memory Review Refactor Plan (Implemented)

## Context

Previously, every retrieval automatically triggered an auto-GOOD review, indiscriminately reinforcing all retrieved memories. This caused self-reinforcing random bias — incorrectly retrieved memories would also be reinforced, making them even easier to retrieve next time.

Refactor goal: remove auto-GOOD, replace with an LLM reviewer at segmentation time that evaluates the actual effectiveness of retrieved memories, updating FSRS parameters with Again/Hard/Good/Easy ratings.

Core principle: **review ≠ retrieval**. Retrieval is search; review is post-hoc evaluation. Only review updates FSRS parameters (including last_reviewed_at).

## Flow

```text
retrieve_memory(query, conversation_id)
  → Normal retrieval returns results (no FSRS parameters updated)
  → Appends to message_queue's pending_reviews column: { memory_ids, query }

Conversation continues (assistant msg, user msg, ...)

When segmentation triggers (rule-based or LLM-decided):
  → Check if message_queue has pending_reviews
  → If yes, enqueue MemoryReviewJob:
      context = full conversation messages
      pending_reviews (passed as-is)
  → Clear pending_reviews (set back to NULL)
  → Execute segmentation normally (create episodic memory)

MemoryReviewJob (async worker):
  → Aggregate pending_reviews: deduplicate by memory_id, record which queries matched each memory
  → Fetch each memory's summary from database
  → LLM evaluates each memory's relevance strength in the conversation context
  → Output Again/Hard/Good/Easy rating
  → Update stability, difficulty, last_reviewed_at using the rating
```

## pending_reviews Data Structure

### Database

New column on message_queue table:

```sql
ALTER TABLE message_queue
ADD COLUMN pending_reviews JSONB DEFAULT NULL;
```

- `NULL` = no pending reviews
- Non-NULL = JSONB array, each retrieval appends one entry

### Structure

```json
[
  {
    "query": "user's Rust learning progress",
    "memory_ids": ["aaa-111", "bbb-222", "ccc-333", "ddd-444", "eee-555"]
  },
  {
    "query": "user's programming preferences",
    "memory_ids": ["bbb-222", "fff-666", "ggg-777", "hhh-888", "iii-999"]
  }
]
```

### Append Operation (at retrieve_memory time)

```sql
UPDATE message_queue
SET pending_reviews = COALESCE(pending_reviews, '[]'::jsonb) || $1::jsonb
WHERE id = $conversation_id;
```

### Aggregation (at review job processing time)

Deduplicate by memory_id, record which queries matched each memory:

```text
memory bbb-222: matched queries ["user's Rust learning progress", "user's programming preferences"]
memory aaa-111: matched queries ["user's Rust learning progress"]
memory fff-666: matched queries ["user's programming preferences"]
...
```

Match count serves as a reference signal for the LLM, not a hard rule.

## Change List

### 1. DB Migration: Add column to message_queue

- `pending_reviews`: `JSONB` (nullable, default null)

### 2. Entity Update

File: `crates/entities/src/message_queue.rs`

- Add `pending_reviews` field

### 3. retrieve_memory API Changes

File: `crates/server/src/api/retrieve_memory.rs`

- Add `conversation_id: Uuid` to request
- Remove `enqueue_review_job` function and its calls
- Replace with: after retrieval, call `MessageQueue::add_pending_review` to append memory_ids + query

### 4. MessageQueue Extension

File: `crates/core/src/message_queue.rs`

- `add_pending_review(id, memory_ids, query, db)` — JSONB concatenate append
- `take_pending_reviews(id, db)` — read and set back to NULL (atomic)

### 5. Segmentation Flow Changes

File: `crates/worker/src/jobs/event_segmentation.rs`

- In `process_event_segmentation`, before creating episodic memory (and on skip paths):
  - Call `MessageQueue::take_pending_reviews`
  - If pending reviews exist, enqueue `MemoryReviewJob`

### 6. MemoryReviewJob Rewrite

File: `crates/worker/src/jobs/memory_review.rs`

```rust
pub struct MemoryReviewJob {
    pub pending_reviews: Vec<PendingReview>,
    pub context_messages: Vec<Message>,
    pub reviewed_at: DateTime<Utc>,
}

pub struct PendingReview {
    pub query: String,
    pub memory_ids: Vec<Uuid>,
}
```

Processing flow:

1. Aggregate pending_reviews → unique memory set (each memory with its matched queries)
2. Fetch each memory's summary from database
3. Build markdown-formatted user message, call LLM review function
4. For each memory, update FSRS parameters based on the LLM's rating:

```rust
let fsrs = FSRS::new(Some(&DEFAULT_PARAMETERS))?;

for (memory_id, rating) in review_results {
    let model = episodic_memory::Entity::find_by_id(memory_id).one(db).await?;

    // Stale skip: if job timestamp is not newer than last review, skip
    if job.reviewed_at <= model.last_reviewed_at { continue; }

    let days_elapsed = (job.reviewed_at - model.last_reviewed_at).num_days();
    let current_state = MemoryState {
        stability: model.stability,
        difficulty: model.difficulty,
    };

    let next_states = fsrs.next_states(
        Some(current_state), DESIRED_RETENTION, days_elapsed
    )?;

    // Select FSRS state based on LLM rating
    let new_state = match rating {
        Rating::Again => next_states.again.memory,
        Rating::Hard  => next_states.hard.memory,
        Rating::Good  => next_states.good.memory,
        Rating::Easy  => next_states.easy.memory,
    };

    // Update database
    active_model.stability = Set(new_state.stability);
    active_model.difficulty = Set(new_state.difficulty);
    active_model.last_reviewed_at = Set(job.reviewed_at.into());
}
```

### 7. LLM Review Function

File: `crates/worker/src/jobs/memory_review.rs`

Structured output, similar pattern to segment_events.

#### System Prompt

```text
You are a memory relevance reviewer. Evaluate how relevant each retrieved memory was to the conversation context.

For each memory, assign a rating:
- "again": Memory was not used in the conversation at all. It is noise.
- "hard": Memory is tangentially related but required significant inference to connect.
- "good": Memory is directly relevant and visibly influenced the conversation.
- "easy": Memory is a core pillar of the conversation. The conversation could not have proceeded meaningfully without it.

Consider:
- Whether the assistant's responses reflect knowledge from the memory
- Whether the memory's content aligns with the conversation topic
- How central the memory is to the conversation flow
- A memory matched by multiple queries may indicate higher relevance, but judge by actual usage in context
```

#### User Message (markdown format)

```markdown
## Conversation Context

- user: "I've been learning Rust lately, the borrow checker is so hard"
- assistant: "You mentioned before that you need to learn Rust within 3 months at your new company. How's it going?"
- user: "Not bad, I've gone through the basic syntax, but lifetimes still confuse me"
- assistant: "Lifetimes are indeed one of the hardest parts of Rust. You said before you prefer learning through projects — want to try building a small CLI tool?"
- user: "Good idea, any recommendations?"

## Retrieved Memories

### Memory aaa-111
**Summary:** User is switching careers from Python to Rust due to performance requirements at new job. Needs to learn within 3 months.
**Matched queries:** "user's Rust learning progress"

### Memory bbb-222
**Summary:** User prefers learning through hands-on projects rather than reading documentation.
**Matched queries:** "user's Rust learning progress", "user's programming preferences"

### Memory ccc-333
**Summary:** User had a casual conversation about weather yesterday.
**Matched queries:** "user's Rust learning progress"
```

#### Structured Output

```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemoryReviewOutput {
    pub ratings: Vec<MemoryRating>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemoryRating {
    /// Memory ID being reviewed
    pub memory_id: String,
    /// Rating: "again", "hard", "good", or "easy"
    pub rating: String,
}
```

Example output:

```json
{
  "ratings": [
    { "memory_id": "aaa-111", "rating": "easy" },
    { "memory_id": "bbb-222", "rating": "good" },
    { "memory_id": "ccc-333", "rating": "again" }
  ]
}
```

### 8. Documentation Updates

- `docs/architecture/fsrs.md`: Update Review section
- `docs/architecture/retrieve_memory.md`: Update Side Effects section
- `AGENTS.md`: Update Key Runtime Flows

## Review Rating Definitions

The LLM judges a single dimension: **relevance strength between the memory and the current conversation**. FSRS automatically translates the rating into memory strength changes.

| Rating | LLM Judgment Criteria | FSRS Effect |
| ------ | --------------------- | ----------- |
| Again | Unrelated to conversation, not used at all | Stability drops significantly |
| Hard | Somewhat related, but requires inference to connect | Stability roughly unchanged |
| Good | Directly relevant, assistant visibly used it | Stability increases moderately |
| Easy | Highly matched, core pillar of the conversation | Stability increases substantially, difficulty decreases |
