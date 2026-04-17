# Semantic Memory

`semantic_memory` stores durable facts extracted from episodes.

Current writes happen only through `PredictCalibrateJob`.

## Schema

Entity:

- `crates/entities/src/semantic_memory.rs`

Fields:

| Field | Purpose |
| --- | --- |
| `id` | fact id |
| `conversation_id` | conversation scope |
| `category` | one of the flat semantic categories |
| `fact` | natural-language fact statement |
| `source_episodic_ids` | provenance |
| `valid_at` | when this fact became valid |
| `invalid_at` | soft invalidation for superseded facts |
| `embedding` | retrieval embedding |
| `created_at` | insertion time |

Current categories:

- `identity`
- `preference`
- `interest`
- `personality`
- `relationship`
- `experience`
- `goal`
- `guideline`

## Write path

```text
EpisodeCreationJob
  -> PredictCalibrateJob
  -> load related facts
  -> cold start extraction OR predict-calibrate extraction
  -> consolidate actions
  -> mark episode consolidated
```

Code:

- `crates/worker/src/jobs/predict_calibrate.rs`

## Action model

The LLM returns semantic actions:

- `new`
- `reinforce`
- `update`
- `invalidate`

Consolidation rules:

- `new`: insert a new fact unless a near-duplicate active fact is found
- `reinforce`: append provenance to an active fact
- `update`: invalidate the target fact, then insert or merge the replacement
- `invalidate`: set `invalid_at`

All writes happen inside one transaction.

## Retrieval

Current semantic retrieval does:

1. BM25 on `fact`
2. vector similarity on `embedding`
3. RRF merge
4. optional category filter

There is no FSRS layer for semantic facts.

Code:

- `crates/core/src/memory/semantic.rs`

## Notes

- The old `subject/predicate/object` model is gone.
- The old `keywords` / generated `search_text` path is gone.
- Current search uses `fact` directly for BM25.
- Invalidated facts stay in the table; retrieval filters to active
  `invalid_at IS NULL` rows.
