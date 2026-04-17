# FSRS

FSRS currently applies only to `episodic_memory`.

`semantic_memory` does not use FSRS.

## Stored state

Per episodic record:

- `stability`
- `difficulty`
- `last_reviewed_at`

## Initialization

Code:

- `crates/worker/src/jobs/episode_creation.rs`

On episode creation, the worker:

1. creates an `FSRS` instance with `DEFAULT_PARAMETERS`
2. calls `next_states(None, DESIRED_RETENTION, 0)`
3. uses the `good` branch as the initial memory state

Current episode creation writes:

- `stability = initial_state.stability`
- `difficulty = initial_state.difficulty`

There is no current surprise-based boost in the write path.

## Retrieval usage

Code:

- `crates/core/src/memory/episodic.rs`

Episodic retrieval computes:

```text
final_score = rrf_score * retrievability
```

Where retrievability comes from:

- current `stability`
- current `difficulty`
- days since `last_reviewed_at`

## Review usage

Code:

- `crates/worker/src/jobs/memory_review.rs`

The review worker:

1. builds `MemoryState { stability, difficulty }`
2. computes `next_states(Some(current_state), DESIRED_RETENTION, days_elapsed)`
3. picks the branch matching the LLM rating
4. updates `stability`, `difficulty`, and `last_reviewed_at`

## What is not implemented

- semantic-memory FSRS
- automatic deletion or archival based on FSRS
- retrieval-time FSRS mutation
