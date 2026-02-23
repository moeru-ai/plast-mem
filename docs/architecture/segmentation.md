# Event Segmentation

Plast Mem implements a **batch event segmentation** system aligned with **Event Segmentation Theory (EST)**. The dual-channel boundary detection design (topic shift + surprise) is inspired by [HiMem](https://arxiv.org/abs/2601.06377).

The system accumulates messages in a per-conversation queue and periodically runs a single LLM call to segment the batch into coherent episodes.

## Architecture

```text
New Message → MessageQueue::push()
                    ↓
             RETURNING jsonb_array_length(messages) → trigger_count
                    ↓
             MessageQueue::check(trigger_count)
                    ↓ (count trigger OR time trigger)
             try_set_fence (CAS) → EventSegmentationJob enqueued
                    ↓
             batch_segment(messages[0..fence_count])  ← single LLM call
                    ↓
        ┌───────────┴────────────┐
        │ 1 segment, not doubled │  → double window, clear fence
        │ 1 segment, doubled     │  → drain + finalize, create 1 episode
        │ N segments             │  → drain N-1, finalize, create N-1 episodes in parallel
        └────────────────────────┘
```

## Trigger Conditions

**Code**: `crates/core/src/message_queue/check.rs`

A segmentation job is triggered when **either** condition is met:

| Condition | Threshold |
| --------- | --------- |
| Count trigger | `trigger_count ≥ WINDOW_BASE` (20) or `WINDOW_MAX` (40) if doubled |
| Time trigger | Oldest message in queue is > 2 hours old |
| Minimum floor | Always skip if `trigger_count < 5` |

### Fence Mechanism (TOCTOU prevention)

`push()` uses `RETURNING jsonb_array_length(messages)` to capture the exact post-push message count. This `trigger_count` is passed directly to `check()` and then to `try_set_fence()`, so the fence boundary is pinned to the triggering message's position — not a re-read that could include later arrivals.

`try_set_fence()` is a CAS operation:

```sql
UPDATE message_queue
SET in_progress_fence = $2, in_progress_since = NOW()
WHERE id = $1 AND in_progress_fence IS NULL
RETURNING id
```

Only one concurrent caller wins; others get 0 rows and bail out.

Stale fences (> 120 minutes) are cleared automatically before trigger evaluation.

## Batch Segmentation (LLM)

**Code**: `crates/core/src/message_queue/segmentation.rs`

A single LLM call (`batch_segment`) receives all messages in the window and returns a list of segments. Each segment includes:

- `start_message_index` / `end_message_index` / `num_messages` — slice boundaries
- `title` — 5–15 word theme description
- `summary` — ≤50 word third-person narrative
- `surprise_level` — `low` | `high` | `extremely_high`

### Boundary Criteria (OR relationship)

1. **Topic shift** — subject, activity, or intent changes; discourse markers ("by the way", "换个话题") and intent reversals (chatting→deciding) count
2. **Surprise** — emotional reversal, domain jump, tone change, or notable time gap

### Surprise Level → FSRS Signal

| Level | Signal |
| ----- | ------ |
| `low` | 0.2 |
| `high` | 0.6 |
| `extremely_high` | 0.9 |

Surprise signal feeds into FSRS stability boost on episode creation:

```rust
boosted_stability = initial_stability * (1.0 + surprise * SURPRISE_BOOST_FACTOR)
```

`extremely_high` (≥ 0.85) also triggers immediate semantic consolidation (flashbulb memory path).

## Window Doubling

If the LLM returns exactly 1 segment (no split detected):

- **First time**: set `window_doubled = true`, clear fence, wait for more messages (window grows to 40)
- **After doubling**: force drain all messages as a single episode

## Drain Order (crash safety)

To prevent duplicate episodes on job retry, the drain order is:

```text
drain + finalize_job  ←── committed first (fence released)
enqueue_pending_reviews
create episodes in parallel  ←── if crash here, messages already gone (acceptable loss)
enqueue semantic consolidation
```

## Edge Message

The **last segment** from a multi-segment result is never drained — it stays in the queue as the start of the next event context. Only `segments[0..N-1]` are drained and converted to episodes.

## Code Locations

| Component | Location |
| --------- | -------- |
| Trigger check + fence | `crates/core/src/message_queue/check.rs` |
| Batch LLM segmentation | `crates/core/src/message_queue/segmentation.rs` |
| Queue push + drain | `crates/core/src/message_queue/mod.rs` |
| Fence + pending reviews state | `crates/core/src/message_queue/state.rs` |
| Job dispatch | `crates/worker/src/jobs/event_segmentation.rs` |
| Episode creation | `crates/core/src/memory/episodic/creation.rs` |
