# Memory Retrieval

Current retrieval exposes three endpoints:

| Endpoint | Purpose |
| --- | --- |
| `POST /api/v0/retrieve_memory` | markdown tool output |
| `POST /api/v0/retrieve_memory/raw` | raw JSON result |
| `POST /api/v0/context_pre_retrieve` | semantic-only markdown, no review side effects |

## Request fields

`retrieve_memory` and `retrieve_memory/raw` accept:

- `conversation_id`
- `query`
- `query_embedding` (optional)
- `episodic_limit`
- `semantic_limit`
- `detail`
- `category`

`context_pre_retrieve` accepts the semantic subset:

- `conversation_id`
- `query`
- `query_embedding` (optional)
- `semantic_limit`
- `detail`
- `category`

## Current retrieval pipeline

```text
embed query if query_embedding is absent
  -> semantic retrieval
  -> episodic retrieval
  -> join results
  -> record pending review item if episodic results exist and review is enabled
```

### Semantic leg

- BM25 on `semantic_memory.fact`
- vector similarity on `embedding`
- RRF merge
- optional category filter

### Episodic leg

- BM25 on `episodic_memory.search_text`
- vector similarity on `embedding`
- RRF merge
- FSRS retrievability multiplier

## Current markdown rendering

Code:

- `crates/core/src/memory/retrieval.rs`

The current formatter is much simpler than the older docs described.

It renders:

```markdown
## Episodic Memories
<episode content block>
<episode content block>

## Known Facts
- <fact>
- <fact>
```

Important current facts:

- episodic markdown uses `mem.content` directly
- semantic markdown uses `fact.fact` directly
- the formatter does not currently render rank, score, details, relative time,
  or surprise labels
- the `detail` field is still in the API but is currently ignored by the
  formatter

## Raw JSON

`retrieve_memory/raw` returns:

- semantic memories plus score
- episodic memories plus score

The memory structs themselves come from:

- `plastmem_core::SemanticMemory`
- `plastmem_core::EpisodicMemory`

## Review side effects

`retrieve_memory` and `retrieve_memory/raw` may call
`add_pending_review_item(...)` when:

- episodic results are not empty
- `ENABLE_FSRS_REVIEW` is enabled

`context_pre_retrieve` never records pending review work.
