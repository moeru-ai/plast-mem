# Memory Retrieval API

The `retrieve_memory` API provides LLM-optimized access to both semantic facts and episodic memories. Episodic retrieval uses hybrid search (BM25 + vector) with FSRS-based re-ranking; semantic retrieval uses hybrid search without FSRS.

## Endpoints

| Endpoint | Method | Response Format | Use Case |
| -------- | ------ | --------------- | -------- |
| `/api/v0/retrieve_memory` | POST | Markdown (tool result) | LLM consumption |
| `/api/v0/retrieve_memory/raw` | POST | JSON | Debug/integration |
| `/api/v0/context_pre_retrieve` | POST | Markdown (semantic only) | System prompt injection |

## Request Format

```json
{
  "query": "what did the user say about Rust",
  "conversation_id": "550e8400-e29b-41d4-a716-446655440001",
  "episodic_limit": 5,
  "semantic_limit": 20,
  "detail": "auto",
  "category": null
}
```

### Parameters

| Field | Type | Default | Description |
| ----- | ---- | ------- | ----------- |
| `query` | string | required | Search query text |
| `conversation_id` | uuid | required | Conversation scope for search and pending review tracking |
| `episodic_limit` | number | 5 | Maximum episodic memories to return (1-100) |
| `semantic_limit` | number | 20 | Maximum semantic facts to return |
| `detail` | string | `"auto"` | Detail level for episodic memories: `"auto"`, `"none"`, `"low"`, `"high"` |
| `category` | string \| null | `null` | Optional semantic category filter (e.g. `"guideline"`, `"preference"`). `null` = all categories |

`context_pre_retrieve` accepts the same fields except `episodic_limit` and `detail` (it only retrieves semantic facts).

## Retrieval Pipeline

Both search legs run in parallel:

```
                          ┌── Semantic Search (BM25 + vector RRF) ──▶ facts (up to semantic_limit)
Query → embed(query) ─────┤
                          └── Episodic Search (BM25 + vector RRF) ──▶ FSRS rerank ──▶ memories (up to episodic_limit)
                                                                                           │
                                                                               Record pending review
```

`context_pre_retrieve` runs only the semantic leg and does **not** record a pending review.

### Episodic: Hybrid Search + FSRS Re-ranking

1. BM25 search on `summary` text → 100 candidates
2. Vector search on `embedding` → 100 candidates
3. RRF fusion: `rrf_score = Σ 1.0 / (60 + rank)`
4. FSRS re-ranking: `final_score = rrf_score × retrievability`
5. Sort by `final_score`, truncate to `episodic_limit`
6. Record pending review in `MessageQueue`

Where `retrievability` is calculated from:
- `stability` — decay rate (boosted by surprise at creation)
- `difficulty` — inherent memorization difficulty
- `days_elapsed` — time since last review

See [FSRS](fsrs.md) for details.

### Semantic: Hybrid Search (no FSRS)

1. BM25 search on `search_text` column (`fact || ' ' || keywords joined`) → 100 candidates
2. Vector search on `embedding` → 100 candidates
3. RRF fusion: `rrf_score = Σ 1.0 / (60 + rank)`
4. Optional category filter: `AND ($category::text IS NULL OR category = $category)`
5. Sort by `rrf_score`, truncate to `semantic_limit`

Semantic facts do not decay and are not subject to FSRS re-ranking. Only active facts (`invalid_at IS NULL`) are searched. BM25 runs against `search_text` (not just `fact`) so that keywords such as entity names contribute to BM25 scoring.

## Response Format (Markdown Endpoint)

The tool result is optimized for LLM consumption. Semantic facts are rendered first as a flat list with `[category]` prefixes, then episodic memories:

```markdown
## Semantic Memory
- [preference] User prefers dark mode interfaces (sources: 2 conversations)
- [experience] User has been doing Python for 5 years (sources: 2 conversations)
- [experience] User's new team uses Rust for a trading system (sources: 1 conversation)
- [guideline] Assistant should emphasize practical examples when teaching (sources: 1 conversation)

## Episodic Memories

### Career switch to Rust [rank: 1, score: 0.92, key moment]
**When:** 2 days ago
**Summary:** User switching careers from Python to Rust due to performance requirements at new job.

**Details:**
- user: "I've been doing Python for 5 years but my new team is all Rust"
- assistant: "That's a big shift. What prompted it?"
- user: "The trading system needs microsecond latency, Python can't cut it"
- user: "Also I need to learn it within 3 months or I'm screwed"

### Dark mode preferences [rank: 2, score: 0.85]
**When:** yesterday
**Summary:** User prefers dark mode interfaces and finds light mode straining.
```

The `## Semantic Memory` section is omitted when no matching facts exist.

### Detail Level Behavior

Applies to episodic memories only. Semantic facts are always rendered as bullet points.

| `detail` | Behavior |
| -------- | -------- |
| `"auto"` | Ranks 1-2 with `surprise ≥ 0.7` get full details |
| `"none"` | No details for any memory (summaries only) |
| `"low"` | Only rank 1 gets details (if surprising) |
| `"high"` | All returned memories get full details |

### Episodic Field Reference

| Field | Source | Description |
| ----- | ------ | ----------- |
| `rank` | Result position | 1-indexed position in results |
| `score` | Final score | `rrf_score × retrievability` (0.0-1.0+) |
| `key moment` | Surprise flag | Present when `surprise ≥ 0.7` |
| `When` | Relative time | Human-readable (e.g., "2 days ago"), derived from `end_at` |
| `Summary` | `summary` | LLM-generated memory summary |
| `Details` | `messages` | Full conversation excerpt (conditional on detail level) |

## Response Format (Raw JSON Endpoint)

Returns a single object with `semantic` and `episodic` arrays:

```json
{
  "semantic": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440002",
      "conversation_id": "550e8400-e29b-41d4-a716-446655440001",
      "category": "preference",
      "fact": "User likes dark mode interfaces.",
      "keywords": ["dark mode"],
      "source_episodic_ids": ["550e8400-e29b-41d4-a716-446655440003"],
      "valid_at": "2025-01-14T09:00:00Z",
      "invalid_at": null,
      "score": 0.031
    }
  ],
  "episodic": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "conversation_id": "550e8400-e29b-41d4-a716-446655440001",
      "messages": [
        {"role": "user", "content": "..."},
        {"role": "assistant", "content": "..."}
      ],
      "title": "Career switch to Rust",
      "summary": "User switching careers from Python to Rust...",
      "stability": 3.5,
      "difficulty": 5.0,
      "surprise": 0.85,
      "start_at": "2025-01-15T10:00:00Z",
      "end_at": "2025-01-15T10:05:00Z",
      "created_at": "2025-01-15T10:05:00Z",
      "last_reviewed_at": "2025-01-15T10:05:00Z",
      "consolidated_at": "2025-01-15T11:00:00Z",
      "score": 0.92
    }
  ]
}
```

Note: `embedding` is omitted from both semantic and episodic responses (`#[serde(skip)]`).

## Design Rationale

### Why Markdown for LLMs?

| Aspect | Markdown | JSON |
| ------ | -------- | ---- |
| Token overhead | Low (~20 tokens) | Medium (~30 tokens) |
| Human readability | Good | Poor |
| LLM familiarity | Very high | High |
| Native sectioning | Headers | Nested braces |

### Selective Detail Inclusion

- **Token efficiency**: Full conversations can consume thousands of tokens
- **Signal-to-noise**: High-surprise memories contain the "key moments"
- **Natural attention**: The `key moment` label guides LLM focus

### Why FSRS Re-ranking for Episodic Only?

Semantic facts are either active or invalidated—they don't decay. FSRS decay modeling is only meaningful for episodic memories, where recency and review history affect how "fresh" a memory is.

### Why BM25 on `search_text` (not `fact`)?

`search_text` is a generated column: `fact || ' ' || array_to_string(keywords, ' ')`. BM25 on this column gives keyword entity names equal weight alongside the fact text. This improves entity recall—e.g., querying "Alex" finds facts that mention Alex in keywords even when "Alex" appears incidentally in the fact sentence.

## Side Effects

Each `retrieve_memory` call records a pending review in `MessageQueue` (episodic memory IDs + query). `context_pre_retrieve` has no side effects. No FSRS parameters are updated at retrieval time.

When event segmentation later triggers, the segmentation worker takes the pending reviews and enqueues a `MemoryReviewJob`. The review worker then uses an LLM to evaluate each memory's relevance in the conversation context and updates FSRS parameters accordingly. See [FSRS](fsrs.md) for rating details.

## Example Scenarios

### Category-filtered: guidelines only

```bash
POST /api/v0/retrieve_memory
{
  "query": "how should I respond",
  "conversation_id": "550e8400-e29b-41d4-a716-446655440001",
  "category": "guideline",
  "semantic_limit": 10
}
```

### System prompt injection (semantic only, no pending review)

```bash
POST /api/v0/context_pre_retrieve
{
  "query": "User is asking about career decisions",
  "conversation_id": "550e8400-e29b-41d4-a716-446655440001"
}
# Returns: markdown of semantic facts only
```

### Deep context needed

```bash
POST /api/v0/retrieve_memory
{
  "query": "what should I learn next",
  "conversation_id": "550e8400-e29b-41d4-a716-446655440001",
  "detail": "high"
}
# Returns: facts + all episodic memories with full message details
```

### Summary only

```bash
POST /api/v0/retrieve_memory
{
  "query": "remind me what we discussed",
  "conversation_id": "550e8400-e29b-41d4-a716-446655440001",
  "detail": "none"
}
# Returns: facts + episode summaries only, no message details
```

## See Also

- [Semantic Memory](semantic_memory.md) — How facts are created and structured
- [Episodic Memory](episodic_memory.md) — How episodes are stored and scored
- [Memory Review](memory_review.md) — FSRS update triggered after retrieval
- [FSRS](fsrs.md) — Retrievability formula details
