# FSRS Integration

## Memory State

Each memory is associated with the following state parameters:

- `stability` (S) — boosted by surprise on creation: `S * (1.0 + surprise * 0.5)`
- `difficulty` (D)
- `last_reviewed_at`

New memories are initialized with **high retrievability but low stability**, meaning they are fresh in memory but haven't been reinforced yet. Surprising events receive a stability boost, making them decay slower.

## Reranking

Each memory retrieval first searches 100 candidates via BM25 + vector (RRF), then applies two multipliers:

1. **FSRS retrievability** — how likely the memory is to be recalled
2. **Boundary boost** — weight based on boundary type:
   - PredictionError: 1.3 + 0.2 × surprise (highest)
   - GoalCompletion: 1.2
   - ContentShift: 1.0 (neutral)
   - TemporalGap: 0.9 (reduced)

Final score: `rrf_score × retrievability × boundary_boost`

The top `limit` items are returned.

## Review

After each memory retrieval, a `MemoryReviewJob` is enqueued with the retrieved memory IDs and the retrieval time (`reviewed_at`).

The worker processes the job asynchronously and updates FSRS parameters.
If the job's `reviewed_at` is not newer than the stored `last_reviewed_at`, the job is skipped to avoid stale writes.

Current behavior is an automatic **GOOD** review for each retrieved memory (being retrieved = reinforcement).

Planned behavior (not implemented yet) is an LLM-based reviewer that evaluates retrieved memories and assigns a rating:

| Rating | Description |
|--------|-------------|
| **Again** | False positive - memory was retrieved but irrelevant or should be ignored |
| **Hard** | Memory provided useful context but required significant inference to apply |
| **Good** | Memory provided core information directly relevant to the query |
| ~~Easy~~ | ~~Exact match - not used~~ |

## Cleanup

TBD, it is expected that an "inactive memories" will be implemented, with permanent deletion occurring after prolonged inactivity.
