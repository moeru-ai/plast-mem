# Plast Mem Architecture Overview

## Overview

Plast Mem is a Rust workspace that turns conversation streams into two memory layers:

- `episodic_memory`: concrete conversation episodes with FSRS state
- `semantic_memory`: durable facts extracted from episodes

The current implementation is queue-and-worker based. Messages are appended to
`conversation_message`, segmentation state is tracked in `segmentation_state`,
workers create `episode_span` and `episodic_memory`, and semantic facts are
consolidated later by `PredictCalibrateJob`.

## Crates

### `plastmem`

Application bootstrap.

- connects to PostgreSQL
- runs all SeaORM migrations
- creates Apalis PostgreSQL job storage
- starts the worker and HTTP server

### `plastmem_core`

Domain logic and DB-facing operations.

- `message_ingest.rs`: append single/batch messages, advance `segmentation_state`,
  and try to claim segmentation work
- `segmentation_state.rs`: claim / recover / commit / abort segmentation state,
  plus `episode_span` access
- `pending_review_queue.rs`: enqueue and consume review work items
- `memory/episodic.rs`: episodic retrieval and FSRS reranking
- `memory/semantic.rs`: semantic retrieval
- `memory/retrieval.rs`: markdown rendering for retrieval endpoints

### `plastmem_entities`

SeaORM entities for the current tables:

- `conversation_message`
- `segmentation_state`
- `episode_span`
- `pending_review_queue`
- `episodic_memory`
- `semantic_memory`

### `plastmem_migration`

Fresh-DB schema snapshot. The migration history has been reset to direct
`create table` migrations for the current schema; legacy upgrade history is no
longer preserved.

### `plastmem_ai`

OpenAI-compatible AI wrapper.

- embeddings: `embed`, `embed_many`
- text generation: `generate_text`
- structured generation: `generate_object`
- utility: `cosine_similarity`

### `plastmem_shared`

Shared bottom-layer types.

- `Message`, `MessageRole`
- `AppError`
- `APP_ENV`

### `plastmem_event_segmentation`

Active segmentation policy crate.

- temporal rule segmentation
- primitive LLM review / split
- informative boundary review / constrained resegmentation
- prompt contracts and output validation

The crate still exports legacy segmentation types, but the active worker path
uses `batch_segmentation.rs`.

### `plastmem_worker`

Background jobs.

- `event_segmentation.rs`: worker orchestration around active segmentation
- `episode_creation.rs`: build `episodic_memory` from `episode_span`
- `memory_review.rs`: FSRS review updates
- `predict_calibrate.rs`: semantic consolidation

### `plastmem_server`

Axum API server and OpenAPI surface.

- message ingestion
- retrieval (`retrieve_memory`, `retrieve_memory/raw`, `context_pre_retrieve`)
- recent episodic memories
- benchmark status endpoint in debug builds

## Runtime Flows

### Message ingestion

```text
POST /api/v0/add_message or /api/v0/import_batch_messages
  -> plastmem_core::append_message / append_batch_messages
  -> write conversation_message rows
  -> update segmentation_state
  -> try_claim_segmentation_job
  -> enqueue EventSegmentationJob if a claim is created
```

### Segmentation to episode creation

```text
EventSegmentationJob
  -> validate active claim
  -> load claimed messages
  -> temporal_rule_segmenter
  -> primitive_review_llm_segmenter
  -> temporal_boundary_review_llm_segmenter
  -> build_commit_plan
  -> commit_segmentation_job
  -> enqueue EpisodeCreationJob
  -> optionally enqueue MemoryReviewJob
  -> optionally enqueue follow-up EventSegmentationJob
```

### Episode creation to semantic consolidation

```text
EpisodeCreationJob
  -> load current EpisodeSpan
  -> ensure episodic_memory exists
  -> generate title/content/embedding + FSRS init
  -> insert episodic_memory if missing
  -> enqueue PredictCalibrateJob when needed

PredictCalibrateJob
  -> load related semantic facts
  -> cold start extraction or predict-calibrate extraction
  -> consolidate semantic actions
  -> mark episodic_memory.consolidated_at
```

### Retrieval and review

```text
POST /api/v0/retrieve_memory or /api/v0/retrieve_memory/raw
  -> embed query
  -> retrieve semantic + episodic in parallel
  -> add pending_review_queue item when episodic results exist and review is enabled

Later, after segmentation commits:
  -> take_pending_review_items
  -> enqueue MemoryReviewJob
  -> update FSRS stability / difficulty / last_reviewed_at
```

## Storage Model

### Raw conversation stream

- `conversation_message` stores the ordered message log
- `segmentation_state` stores unsegmented progress and active claim metadata

### Segmentation artifacts

- `episode_span` stores committed segment ranges and their classification

### Memory layers

- `episodic_memory` stores rendered episode content, FSRS state, and
  consolidation status
- `semantic_memory` stores active and invalidated facts with provenance

## Notes

- `surprise` still exists in `episodic_memory`, but current
  `episode_creation.rs` writes `0.0` for new records.
- `retrieve_memory` still accepts `detail`, but the current markdown renderer
  does not branch on it.
- Benchmark-only routes live behind `debug_assertions`.

## Further Reading

- [Segmentation](architecture/segmentation.md)
- [Episodic Memory](architecture/episodic_memory.md)
- [Semantic Memory](architecture/semantic_memory.md)
- [Memory Retrieval](architecture/retrieve_memory.md)
- [Memory Review](architecture/memory_review.md)
- [FSRS](architecture/fsrs.md)
