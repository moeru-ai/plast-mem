# Memory Retrieval API

The `retrieve_memory` API provides LLM-optimized access to episodic memories with hybrid search (BM25 + vector) and FSRS-based re-ranking.

## Endpoints

| Endpoint | Method | Response Format | Use Case |
|----------|--------|-----------------|----------|
| `/api/v0/retrieve_memory` | POST | Markdown (tool result) | LLM consumption |
| `/api/v0/retrieve_memory/raw` | POST | JSON | Debug/integration |

## Request Format

```json
{
  "query": "what did the user say about Rust",
  "conversation_id": "550e8400-e29b-41d4-a716-446655440001",
  "limit": 5,
  "detail": "auto"
}
```

### Parameters

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `query` | string | required | Search query text |
| `conversation_id` | uuid | required | Conversation ID for pending review tracking |
| `limit` | number | 5 | Maximum memories to return (1-100) |
| `detail` | string | "auto" | Detail level: `"auto"`, `"none"`, `"low"`, `"high"` |

## Retrieval Pipeline

```
Query → Embedding → BM25 Search (100 candidates)
                    Vector Search (100 candidates)
                           ↓
                     RRF Fusion (score merging)
                           ↓
              FSRS Re-ranking (× retrievability)
                           ↓
              Sort by final_score, truncate to limit
                           ↓
              Record pending review in MessageQueue
```

### Hybrid Search (RRF)

Reciprocal Rank Fusion combines BM25 and vector search results:

```
rrf_score = Σ 1.0 / (60 + rank)
```

Each memory appearing in both result sets gets a higher combined score.

### FSRS Re-ranking

Final score incorporates memory "freshness" using FSRS retrievability:

```
final_score = rrf_score × retrievability
```

Where `retrievability` is calculated from:
- `stability` — decay rate (boosted by surprise at creation)
- `difficulty` — inherent memorization difficulty
- `days_elapsed` — time since last review

See [FSRS](fsrs.md) for details.

## Response Format (Markdown Endpoint)

The tool result format is optimized for LLM consumption:

```markdown
## Memory 1 [rank: 1, score: 0.92, key moment]
**When:** 2 days ago
**Summary:** User switching careers from Python to Rust due to performance requirements at new job.

**Details:**
- user: "I've been doing Python for 5 years but my new team is all Rust"
- assistant: "That's a big shift. What prompted it?"
- user: "The trading system needs microsecond latency, Python can't cut it"
- user: "Also I need to learn it within 3 months or I'm screwed"

## Memory 2 [rank: 2, score: 0.85]
**When:** yesterday
**Summary:** User prefers dark mode interfaces and finds light mode straining.
```

### Detail Level Behavior

| `detail` | Behavior |
|----------|----------|
| `"auto"` | Ranks 1-2 with `surprise ≥ 0.7` get full details |
| `"none"` | No details for any memory (summaries only) |
| `"low"` | Only rank 1 gets details (if surprising) |
| `"high"` | All returned memories get full details |

### Field Reference

| Field | Source | Description |
|-------|--------|-------------|
| `rank` | Result position | 1-indexed position in results |
| `score` | Final score | `rrf_score × retrievability` (0.0-1.0+) |
| `key moment` | Surprise flag | Present when `surprise ≥ 0.7` |
| `When` | Relative time | Human-readable (e.g., "2 days ago") |
| `Summary` | `content` | LLM-generated memory summary |
| `Details` | `messages` | Full conversation excerpt (conditional) |

## Response Format (Raw JSON Endpoint)

```json
[
  {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "conversation_id": "550e8400-e29b-41d4-a716-446655440001",
    "messages": [
      {"role": "user", "content": "..."},
      {"role": "assistant", "content": "..."}
    ],
    "content": "Summary text",
    "embedding": [0.1, 0.2, ...],
    "stability": 3.5,
    "difficulty": 5.0,
    "surprise": 0.85,
    "start_at": "2025-01-15T10:00:00Z",
    "end_at": "2025-01-15T10:05:00Z",
    "created_at": "2025-01-15T10:05:00Z",
    "last_reviewed_at": "2025-01-15T10:05:00Z",
    "score": 0.92
  }
]
```

## Design Rationale

### Why Markdown for LLMs?

| Aspect | Markdown | JSON |
|--------|----------|------|
| Token overhead | Low (~20 tokens) | Medium (~30 tokens) |
| Human readability | Good | Poor |
| LLM familiarity | Very high | High |
| Native sectioning | Headers | Nested braces |

### Selective Detail Inclusion

- **Token efficiency**: Full conversations can consume thousands of tokens
- **Signal-to-noise**: High-surprise memories contain the "key moments"
- **Natural attention**: The `key moment` label guides LLM focus

### Why FSRS Re-ranking?

Memories decay over time. A highly relevant but ancient memory may be less useful than a moderately relevant recent one. FSRS models this decay and adjusts scores accordingly.

## Side Effects

Each retrieval records a pending review in `MessageQueue` (memory IDs + query). No FSRS parameters are updated at retrieval time.

When event segmentation later triggers, the segmentation worker takes the pending reviews and enqueues a `MemoryReviewJob`. The review worker then uses an LLM to evaluate each memory's relevance in the conversation context and updates FSRS parameters accordingly. See [FSRS](fsrs.md) for rating details.

## Example Scenarios

### Casual Query

```bash
POST /api/v0/retrieve_memory
{ "query": "how are you" }

# Returns: 5 summaries, 0 details (no memories qualify as key moments)
```

### Deep Context Needed

```bash
POST /api/v0/retrieve_memory
{ "query": "what should I learn next", "detail": "high" }

# Returns: all memories with full details
```

### Explicit Summary Only

```bash
POST /api/v0/retrieve_memory
{ "query": "remind me what we discussed", "detail": "none" }

# Returns: 5 summaries only, no details for any
```

### Minimal Detail

```bash
POST /api/v0/retrieve_memory
{ "query": "quick reminder", "detail": "low" }

# Returns: only rank 1 gets details (if surprising)
```
