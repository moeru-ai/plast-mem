# Episodic Memory

Episodic memory stores conversation segments as discrete events, representing "what happened" in specific contexts. It is the primary memory type for contextual retrieval.

## Overview

Unlike semantic memory (facts) or procedural memory (skills), episodic memory captures concrete experiences with temporal boundaries—conversations, interactions, and events that occurred at specific times.

```
Conversation Messages → Event Segmentation → EpisodicMemory (with FSRS state)
                              ↓
                        Surprise Detection
                              ↓
                   Stability Boost (1.0 + surprise × 0.5)
                              ↓
                   (if threshold/flashbulb) → SemanticConsolidation
```

## Schema

- **Core struct**: `crates/core/src/memory/episodic.rs`
- **Episode creation**: `crates/worker/src/jobs/event_segmentation.rs`
- **Database entity**: `crates/entities/src/episodic_memory.rs`

### Field Semantics

| Field | Purpose | Mutable |
|-------|---------|---------|
| `id` | Primary key | No |
| `conversation_id` | Grouping key for multi-conversation isolation | No |
| `messages` | Source conversation (preserved for detail views) | No |
| `title` | Short episode title (5-15 words, LLM-generated) | No |
| `summary` | Searchable narrative summary (LLM-generated) | No |
| `embedding` | Vector of `summary` text for semantic search | No |
| `stability` | FSRS decay parameter | Yes (reviews update) |
| `difficulty` | FSRS difficulty parameter | Yes (reviews update) |
| `surprise` | Creation-time significance score | No |
| `start_at` / `end_at` | Temporal boundaries of the episode | No |
| `created_at` | Record creation | No |
| `last_reviewed_at` | Last retrieval / review | Yes |
| `consolidated_at` | When processed into semantic memory; `NULL` = pending consolidation | Yes |

## Lifecycle

### 1. Creation (Event Segmentation)

Episodic memories are created via the [segmentation](segmentation.md) pipeline:

1. **Rule check** — Fast path for obvious cases (e.g., < 3 messages = no split)
2. **Dual-channel boundary detection** — Surprise channel (embedding divergence) + Topic channel (embedding pre-filter → LLM)
3. **Episode generation** — LLM generates title + summary
4. **FSRS initialization** — Initial stability boosted by surprise signal

```rust
// Stability boost formula
let boosted_stability = base_stability * (1.0 + surprise * 0.5);
// surprise 1.0 → 1.5x stability (slower decay)
// surprise 0.0 → 1.0x stability (normal decay)
```

### 2. Storage

Stored in PostgreSQL with `pgvector` extension:
- BM25 index on `summary` for full-text search
- HNSW index on `embedding` for vector similarity

### 3. Retrieval

See [retrieve_memory](retrieve_memory.md) for detailed API documentation.

Brief pipeline:
1. Hybrid search (BM25 + vector) with RRF fusion → 100 candidates
2. FSRS re-ranking: `final_score = rrf_score × retrievability`
3. Sort and truncate to limit
4. Enqueue `MemoryReviewJob` for async FSRS update

### 4. Review (FSRS Update)

Each retrieval records pending reviews; when event segmentation triggers, a `MemoryReviewJob` evaluates relevance:
- LLM assigns Again/Hard/Good/Easy ratings based on actual usage in conversation
- Updates `stability`, `difficulty`, `last_reviewed_at`

See [Memory Review](memory_review.md) and [FSRS](fsrs.md) for details.

### 5. Semantic Consolidation

After creation, if `consolidated_at IS NULL` and the unconsolidated episode count reaches the threshold (or surprise triggers a flashbulb), a `SemanticConsolidationJob` extracts long-term facts. On completion, `consolidated_at` is set.

See [Semantic Memory](semantic_memory.md) for details.

## Surprise Detection

Surprise measures prediction error—the unexpectedness of information relative to the current event model. It is computed as `1 - cosine_sim(event_model_embedding, new_message_embedding)` during boundary detection. It serves two purposes:

### 1. FSRS Stability Boost

Higher surprise → higher initial stability → slower decay. Rationale: surprising events contain more learning value and warrant longer retention.

### 2. Display Significance

High-surprise memories are labeled "key moment" in tool results and may include full message details based on detail level settings.

### Scoring Scale

| Score | Interpretation | Example |
|-------|---------------|---------|
| 0.0 | Fully expected, no new information | "Got it" / "Understood" |
| 0.3 | Minor information gain | "I see" / "Makes sense" |
| 0.7 | Significant pivot or revelation | "Wait, what?" / "That's different" |
| 1.0 | Complete surprise, model-breaking | "I had no idea" / "That changes everything" |

### Why Surprise Over Valence?

| Aspect | Valence (positive/negative) | Surprise |
|--------|---------------------------|----------|
| Memory relevance | Low (emotion ≠ importance) | High (unexpected = learning) |
| Detection cost | Sentiment analysis | Single scale, objective |
| EST alignment | Weak | Strong (implements prediction error) |

## Access Patterns

### Via API

See [retrieve_memory](retrieve_memory.md) for endpoint details.

| Endpoint | Location |
| -------- | -------- |
| `POST /api/v0/retrieve_memory` | `crates/server/src/api/retrieve_memory.rs` |
| `POST /api/v0/retrieve_memory/raw` | `crates/server/src/api/retrieve_memory.rs` |
| `POST /api/v0/recent_memory` | `crates/server/src/api/recent_memory.rs` |
| `POST /api/v0/recent_memory/raw` | `crates/server/src/api/recent_memory.rs` |

### Programmatic

| Operation | Location |
|-----------|----------|
| `EpisodicMemory::retrieve()` | `crates/core/src/memory/episodic.rs` |
| `EpisodicMemory::from_model()` | `crates/core/src/memory/episodic.rs` |
| `EpisodicMemory::to_model()` | `crates/core/src/memory/episodic.rs` |

## Design Decisions

### Why Store Full Messages?

While `summary` is used for search, `messages` preserves the original conversation for detail views. This supports:
- Audit/debugging (see what actually happened)
- High-detail tool results for key moments
- Potential future re-summarization

### Why Separate `start_at` and `end_at`?

Temporal boundaries enable:
- Duration calculation (how long did this interaction take?)
- Temporal queries (memories from morning vs evening)
- Future: temporal decay alongside FSRS decay

### Why FSRS for Memory?

Traditional TTL (time-to-live) or LRU (least-recently-used) don't model human memory:
- TTL deletes regardless of importance
- LRU ignores that some memories should persist even if old

FSRS models retrievability—how likely you are to recall something given when you last reviewed it. This naturally balances relevance and recency.

## Thresholds Reference

| Threshold | Usage |
|-----------|-------|
| `surprise ≥ 0.7` | Key moment flag in tool results |
| `rank ≤ 2` | Eligible for details in `auto` detail level |

## Relationships

```
┌─────────────────┐     ┌──────────────────┐     ┌─────────────────┐
│  MessageQueue   │────▶│ EventSegmentation│────▶│ EpisodicMemory  │
│  (pending)      │     │ (LLM analysis)   │     │ (stored)        │
└─────────────────┘     └──────────────────┘     └─────────────────┘
                                                           │
                           ┌───────────────────────────────┤
                           │                               │ (threshold/flashbulb)
                           ▼                               ▼
                    ┌─────────────────┐     ┌──────────────────────┐
                    │ retrieve_memory │◀─── │ SemanticConsolidation│
                    │ (hybrid search) │     │ (facts extraction)   │
                    └─────────────────┘     └──────────────────────┘
                           │
                           ▼
                    ┌─────────────────┐
                    │ MemoryReviewJob │───▶ FSRS update
                    │ (async worker)  │
                    └─────────────────┘
```

## See Also

- [Segmentation](segmentation.md) — How conversations become memories
- [Semantic Memory](semantic_memory.md) — Long-term facts extracted from episodes
- [FSRS](fsrs.md) — Spaced repetition mechanics
- [Retrieve Memory](retrieve_memory.md) — API for memory access
