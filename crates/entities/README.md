# plastmem_entities

Sea-ORM entities for database tables.

## Overview

This crate contains the database schema definitions as Sea-ORM entities.
Entities are manually maintained alongside migrations in the `migration` crate.

## Entities

### [episodic_memory](src/episodic_memory.rs)

Stores episodic memories with FSRS parameters (stability, difficulty) for spaced repetition scheduling.

Key fields: `id`, `conversation_id`, `messages`, `title`, `summary`, `embedding`, `stability`, `difficulty`, `surprise`, `start_at`, `end_at`, `consolidated_at`.

### [semantic_memory](src/semantic_memory.rs)

Stores categorized long-term facts with temporal validity tracking.

Key fields: `id`, `conversation_id`, `category`, `fact`, `keywords`, `source_episodic_ids`, `valid_at`, `invalid_at`, `embedding`.

The `search_text` generated column exists in the DB (used for the BM25 index: `fact || ' ' || keywords`) but is not mapped in the entity since it cannot be inserted or updated.

### [message_queue](src/message_queue.rs)

Per-conversation message buffer and segmentation state.

Key fields: `id`, `messages`, `pending_reviews`, `in_progress_fence`, `in_progress_since`, `window_doubled`, `prev_episode_summary`.

## Updating Entities

After schema changes, update the entity files manually to match the migration:

1. Apply the migration (`cargo run` will auto-migrate on startup)
2. Edit `crates/entities/src/<table>.rs` to add/remove fields
3. Run `cargo check -p plastmem_entities` to verify

The `sea-orm-cli generate entity` command can be used as a starting point but will overwrite manual customizations (like `PgVector` types), so prefer manual edits.
