# plastmem_entities

SeaORM entities for the active Plast Mem schema.

## Tables

### `conversation_message`

Primary key: `(conversation_id, seq)`

Stores the canonical ordered message stream for each conversation.

### `segmentation_state`

Primary key: `conversation_id`

Tracks:

- `last_message_seq`
- `eof_identified`
- `next_segment_start_seq`
- active claim fields (`active_segment_start_seq`, `active_segment_end_seq`, `active_since`)

### `episode_span`

Primary key: `(conversation_id, start_seq)`

Stores committed segment ranges and `EpisodeClassification`.

### `pending_review_queue`

Primary key: `id`

Stores retrieval-time pending review work items.

### `episodic_memory`

Primary key: `id`

Stores:

- source `messages`
- rendered `content`
- `title`
- `embedding`
- FSRS fields (`stability`, `difficulty`, `last_reviewed_at`)
- `surprise`
- optional `classification`
- `consolidated_at`

### `semantic_memory`

Primary key: `id`

Stores:

- `category`
- `fact`
- `source_episodic_ids`
- `valid_at` / `invalid_at`
- `embedding`

## Notes

- `EpisodeClassification` is defined in `episode_classification.rs`.
- Entities are maintained manually alongside `crates/migration`.
