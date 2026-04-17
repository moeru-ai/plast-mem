# plastmem_core

Core domain logic for Plast Mem.

## Scope

This crate owns:

- message ingestion into `conversation_message`
- segmentation state, claim, commit, and stale recovery
- pending review queue operations
- episodic retrieval
- semantic retrieval
- shared retrieval markdown rendering

It does not own:

- HTTP routing
- worker runtime orchestration
- segmentation policy prompts

## Main modules

### `message_ingest.rs`

- `append_message`
- `append_batch_messages`
- `try_claim_segmentation_job`
- `get_claim_messages`

This is the hot-path write side for incoming messages.

### `segmentation_state.rs`

- `get_segmentation_state`
- `recover_stale_segmentation_job`
- `commit_segmentation_job`
- `abort_segmentation_job`
- `get_episode_span`
- `get_messages_in_range`

This module owns the `segmentation_state` and `episode_span` tables.

### `pending_review_queue.rs`

- `add_pending_review_item`
- `take_pending_review_items`

This replaces the old queue-embedded pending-review storage.

### `memory/episodic.rs`

Hybrid episodic retrieval:

- BM25 on `episodic_memory.search_text`
- vector similarity on `embedding`
- RRF merge
- FSRS retrievability reranking

### `memory/semantic.rs`

Hybrid semantic retrieval:

- BM25 on `semantic_memory.fact`
- vector similarity on `embedding`
- RRF merge
- optional category filter

### `memory/retrieval.rs`

Shared markdown formatter for retrieval endpoints.

## Notes

- `DetailLevel` is still part of the retrieval API, but the current formatter
  renders episodic `content` blocks directly and does not branch on `detail`.
- `surprise` is still part of episodic records, but current episode creation
  initializes it to `0.0`.
