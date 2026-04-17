# Change Guide

This guide maps common changes to the current code layout.

## Quick Reference

| Change | Primary files |
| --- | --- |
| Add or change an API endpoint | `crates/server/src/api/*.rs`, `crates/server/src/api/mod.rs` |
| Change message ingestion or segmentation claims | `crates/core/src/message_ingest.rs`, `crates/core/src/segmentation_state.rs` |
| Change active segmentation policy | `crates/event_segmentation/src/batch_segmentation.rs`, `crates/worker/src/jobs/event_segmentation.rs` |
| Change episode creation | `crates/worker/src/jobs/episode_creation.rs` |
| Change FSRS review behavior | `crates/worker/src/jobs/memory_review.rs`, `crates/core/src/memory/episodic.rs` |
| Change semantic consolidation | `crates/worker/src/jobs/predict_calibrate.rs`, `crates/core/src/memory/semantic.rs` |
| Change schema | `crates/entities/src/*.rs`, `crates/migration/src/*.rs` |
| Change TypeScript benchmark/example code | `docs/TYPESCRIPT.md`, `benchmarks/*`, `examples/*` |

## Common Change Patterns

### API changes

Flow:

```text
api handler -> core call(s) -> optional job enqueue -> response
```

Checklist:

1. Update request/response types in the handler file.
2. Keep business logic out of `plastmem_server`.
3. Register the route in `crates/server/src/api/mod.rs`.
4. If the endpoint changes memory state, follow the existing core entrypoints
   instead of writing DB logic in the handler.

Reference files:

- `crates/server/src/api/add_message.rs`
- `crates/server/src/api/retrieve_memory.rs`
- `crates/server/src/api/recent_memory.rs`

### Message ingestion or segmentation state changes

Current segmentation is state-based, not `message_queue` based.

Relevant code:

- `crates/core/src/message_ingest.rs`
- `crates/core/src/segmentation_state.rs`
- `crates/worker/src/jobs/event_segmentation.rs`
- `crates/event_segmentation/src/batch_segmentation.rs`

Typical questions to answer before editing:

1. Does the change affect claim creation?
2. Does it affect stale claim recovery?
3. Does it affect commit semantics (`next_segment_start_seq`, finalized spans)?
4. Does it affect follow-up job enqueueing?

### Active segmentation policy changes

There are two layers:

- `plastmem_event_segmentation`: policy, prompts, structural validation
- `plastmem_worker::event_segmentation`: runtime orchestration, commit, side effects

Use this rule:

- boundary or classification logic -> `batch_segmentation.rs`
- claim validation / commit / enqueueing -> `event_segmentation.rs`

### Episode creation changes

Relevant files:

- `crates/worker/src/jobs/episode_creation.rs`
- `crates/core/src/segmentation_state.rs`
- `crates/entities/src/episodic_memory.rs`

Current flow:

```text
EpisodeSpan
  -> EpisodeCreationJob
  -> ensure episodic_memory exists
  -> generate artifacts
  -> insert episodic_memory
  -> optionally enqueue PredictCalibrateJob
```

If you change episode fields, update:

1. entity
2. migration
3. `create_episode_record`
4. retrieval or downstream jobs if they read the field

### Retrieval changes

Relevant files:

- `crates/core/src/memory/episodic.rs`
- `crates/core/src/memory/semantic.rs`
- `crates/core/src/memory/retrieval.rs`
- `crates/server/src/api/retrieve_memory.rs`

Current behavior to remember:

- episodic retrieval: BM25 on `search_text` + vector + FSRS rerank
- semantic retrieval: BM25 on `fact` + vector, no FSRS
- `detail` is still part of the API, but the current markdown formatter ignores it
- pending review recording happens in `retrieve_memory.rs`, not inside the memory models

### FSRS review changes

Relevant files:

- `crates/worker/src/jobs/memory_review.rs`
- `crates/core/src/memory/episodic.rs`
- `docs/architecture/fsrs.md`

Current review path:

```text
retrieve_memory
  -> add_pending_review_item
  -> segmentation commit
  -> take_pending_review_items
  -> MemoryReviewJob
  -> FSRS next_states update
```

### Semantic consolidation changes

Relevant files:

- `crates/worker/src/jobs/predict_calibrate.rs`
- `crates/core/src/memory/semantic.rs`
- `crates/entities/src/semantic_memory.rs`

Current model:

- no direct write API for semantic facts
- all writes go through `PredictCalibrateJob`
- invalidation uses `invalid_at`, not hard delete

### Schema changes

Migration history has been reset. The current `crates/migration` crate is a
fresh-DB schema snapshot, not a compatibility chain for old databases.

That means:

- changing table shape requires updating the create migrations directly
- existing development databases are expected to be recreated

Checklist:

1. Update the entity in `crates/entities/src`.
2. Update the corresponding create migration in `crates/migration/src`.
3. Update all code that constructs or reads the model.
4. Run at least:

```bash
cargo check -p plastmem_entities
cargo check -p plastmem_migration
cargo check -p plastmem
```

### TypeScript benchmark / example changes

This workspace uses `pnpm`, not `npm`.

Relevant docs and files:

- `docs/TYPESCRIPT.md`
- `benchmarks/locomo`
- `benchmarks/longmemeval`
- `examples/haru`

For CLI-style TypeScript files, match the pattern already used in
`benchmarks/locomo/src/cli.ts`.

## Architecture Rules

### Dependency directions

```text
server -> core
worker -> core
worker -> event_segmentation
core -> entities / shared
ai -> shared
```

Avoid:

- server depending on worker internals
- core depending on server or worker
- worker putting segmentation policy back into runtime orchestration

### Data ownership

- `conversation_message` is the source-of-truth message log
- `segmentation_state` owns unsegmented progress
- `episode_span` owns committed ranges
- `episodic_memory` owns rendered episode artifacts + FSRS state
- `semantic_memory` owns durable facts

## Pitfalls

1. Do not reintroduce `message_queue` assumptions into docs or code.
2. Do not document `detail` behavior that the current renderer does not implement.
3. Do not treat migration history as an upgrade path anymore.
4. `surprise` still exists in schema, but current episode creation initializes it to `0.0`.
5. `predict_calibrate` enqueueing is intentionally retry-tolerant; avoid documenting it as strong queue deduplication.
