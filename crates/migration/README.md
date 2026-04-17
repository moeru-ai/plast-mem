# plastmem_migration

SeaORM migrations for the current Plast Mem schema.

## Current model

Migration history has been reset. This crate now describes the current schema as
a fresh-DB snapshot, not a compatibility chain for older databases.

Current tables:

- `conversation_message`
- `segmentation_state`
- `episode_span`
- `pending_review_queue`
- `episodic_memory`
- `semantic_memory`

## Files

| File | Purpose |
| --- | --- |
| `m20260417_01_create_conversation_message_table.rs` | raw message log |
| `m20260417_02_create_segmentation_state_table.rs` | segmentation progress and active claim |
| `m20260417_03_create_episode_span_table.rs` | committed segment ranges |
| `m20260417_04_create_pending_review_queue_table.rs` | pending FSRS review items |
| `m20260417_05_create_episodic_memory_table.rs` | episodic memories, FSRS state, search index |
| `m20260417_06_create_semantic_memory_table.rs` | semantic facts and indexes |

## Requirements

The migrations assume the database already supports:

- `vector(1024)` / pgvector
- ParadeDB `bm25` indexing and `pdb.icu`

These migrations do not create extensions for you.

## Development note

Because the history is reset:

- editing schema means editing the create migrations directly
- old development databases are expected to be recreated
