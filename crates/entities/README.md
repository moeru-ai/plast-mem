# plastmem_entities

Sea-ORM entities for database tables.

## Overview

This crate contains the database schema definitions as Sea-ORM entities.
Entities are manually maintained alongside migrations in the `migration` crate.

## Entities

### episodic_memory

Stores episodic memories with FSRS parameters:

```rust
pub struct Model {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub messages: Json,           // Vec<Message> as JSON
    pub title: String,
    pub summary: String,
    pub embedding: PgVector,      // pgvector extension
    pub stability: f32,           // FSRS stability
    pub difficulty: f32,          // FSRS difficulty
    pub surprise: f32,            // Creation-time surprise signal
    pub start_at: DateTimeWithTimeZone,
    pub end_at: DateTimeWithTimeZone,
    pub created_at: DateTimeWithTimeZone,
    pub last_reviewed_at: DateTimeWithTimeZone,
    pub consolidated_at: Option<DateTimeWithTimeZone>,
}
```

### semantic_memory

Stores categorized long-term facts:

```rust
pub struct Model {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub category: String,         // One of 8 fixed categories
    pub fact: String,             // Natural language sentence
    pub keywords: Vec<String>,    // Key entity names for BM25 recall
    // search_text is a GENERATED column (fact || ' ' || keywords), not mapped here
    pub source_episodic_ids: Vec<Uuid>,
    pub valid_at: DateTimeWithTimeZone,
    pub invalid_at: Option<DateTimeWithTimeZone>,
    pub embedding: PgVector,
    pub created_at: DateTimeWithTimeZone,
}
```

The `search_text` generated column exists in the DB (used for the BM25 index) but is not mapped in the entity since it cannot be inserted or updated.

### message_queue

Per-conversation message buffer and state:

```rust
pub struct Model {
    pub id: Uuid,                 // conversation_id
    pub messages: Json,           // Vec<Message> as JSON
    pub pending_reviews: Option<Json>, // Vec<PendingReview>
    pub event_model: Option<String>,   // Current event description
    pub last_embedding: Option<PgVector>,
    pub event_model_embedding: Option<PgVector>,
}
```

## Updating Entities

After schema changes, update the entity files manually to match the migration:

1. Apply the migration (`cargo run` will auto-migrate on startup)
2. Edit `crates/entities/src/<table>.rs` to add/remove fields
3. Run `cargo check -p plastmem_entities` to verify

The `sea-orm-cli generate entity` command can be used as a starting point but will overwrite manual customizations (like `PgVector` types), so prefer manual edits.
