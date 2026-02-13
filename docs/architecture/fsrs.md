# FSRS Integration

## Memory State

Each memory is associated with the following state parameters:

- `stability` (S) — boosted by surprise on creation: `S * (1.0 + surprise * 0.5)`
- `difficulty` (D)
- `last_reviewed_at`

New memories are initialized with **high retrievability but low stability**, meaning they are fresh in memory but haven't been reinforced yet. Surprising events receive a stability boost, making them decay slower.

## Reranking

Each memory retrieval first searches 100 candidates via BM25 + vector (RRF), then applies the FSRS retrievability multiplier:

```
final_score = rrf_score × retrievability
```

The top `limit` items are returned.

## Review

Review is decoupled from retrieval. Retrieval only records pending reviews in `MessageQueue`; FSRS parameters are never updated at retrieval time.

When event segmentation triggers, the segmentation worker checks for pending reviews and enqueues a `MemoryReviewJob`. The review worker then:

1. Aggregates pending reviews (deduplicates memory IDs, collects matched queries)
2. Calls an LLM reviewer with the conversation context + retrieved memory summaries
3. Updates FSRS parameters (stability, difficulty, last_reviewed_at) based on the rating

If the job's `reviewed_at` is not newer than the stored `last_reviewed_at`, the update is skipped to avoid stale writes.

| Rating    | Description                                                      | FSRS Effect                        |
|-----------|------------------------------------------------------------------|------------------------------------|
| **Again** | Memory was noise — not used in the conversation at all           | Stability drops significantly      |
| **Hard**  | Tangentially related, required significant inference to connect  | Stability roughly unchanged        |
| **Good**  | Directly relevant, visibly influenced the conversation           | Stability increases moderately     |
| **Easy**  | Core pillar of the conversation, essential to its flow           | Stability increases substantially  |

## Cleanup

TBD, it is expected that an "inactive memories" will be implemented, with permanent deletion occurring after prolonged inactivity.
