# Semantic Memory

Semantic memory stores long-term facts and behavioral guidelines extracted from conversations. Where episodic memory captures *what happened*, semantic memory captures *what is known*—durable knowledge that persists and evolves across many conversations.

## Overview

```
EpisodicMemory (created)
        │
        │  real-time trigger: each new episode
        ▼
PredictCalibrateJob (per episode)
        │
        │  1. PREDICT: Generate content prediction from existing knowledge
        │  2. CALIBRATE: Compare prediction with actual messages
        │  3. Extract high-value knowledge from gaps
        │  4. Consolidate with existing facts
        │  5. Mark episode consolidated
        ▼
SemanticMemory (categorized facts)
```

### Predict-Calibrate Learning (PCL)

This implementation follows Nemori's Predict-Calibrate Learning principle ([arXiv:2508.03341](https://arxiv.org/abs/2508.03341), [GitHub](https://github.com/nemori-ai/nemori)):

**PREDICT Phase**: Generate a prediction of episode content from existing semantic knowledge. The system predicts what the conversation should contain based on known facts.

**CALIBRATE Phase**: Compare the predicted content with actual messages to identify knowledge gaps. High-value facts are extracted from prediction errors (where reality differed from expectations).

**Cold Start**: When no existing knowledge exists, the system uses a specialized extraction prompt to identify high-value facts directly from the first episode.

This approach treats prediction errors as learning signals—when the system's expectations don't match reality, it extracts new knowledge from these discrepancies.

## Schema

- **Core struct**: `crates/core/src/memory/semantic.rs`
- **Database entity**: `crates/entities/src/semantic_memory.rs`

### Field Semantics

| Field | Purpose | Mutable |
|-------|---------|---------|
| `id` | UUID v7 primary key | No |
| `conversation_id` | Isolation boundary — facts scoped per conversation | No |
| `category` | One of 8 fixed categories (see below) | No |
| `fact` | Natural language sentence describing the fact | No |
| `keywords` | Key entity names / nouns for BM25 multi-hop search | No |
| `search_text` | `GENERATED` column: `fact || ' ' || array_to_string(keywords, ' ')` — indexed for BM25 | No (generated) |
| `source_episodic_ids` | UUIDs of episodes that evidence this fact (provenance) | Yes (reinforce appends) |
| `valid_at` | When this fact became valid | No |
| `invalid_at` | When this fact was superseded/contradicted; `NULL` = still active | Yes (Update/Invalidate sets) |
| `embedding` | Vector of `"{category}: {fact} {keywords}"` (category prefix biases embedding) | No |
| `created_at` | Record creation timestamp | No |

### Differences from Episodic Memory

Semantic memory has **no FSRS parameters** (no stability, difficulty, last_reviewed_at). It does not decay. Facts are either active (`invalid_at IS NULL`) or superseded. Instead of time-based retrievability, relevance is determined purely by hybrid search ranking.

## 8 Flat Categories

Every fact is assigned exactly one category. These replace the old SPO predicate taxonomy.

| Category | What it captures |
|----------|-----------------|
| `identity` | Name, location, occupation, age, demographic facts |
| `preference` | Likes, dislikes, favorites, rankings |
| `interest` | Topics, hobbies, domains the person engages with |
| `personality` | Communication style, emotional tendencies, traits |
| `relationship` | Dynamics between user and assistant, shared references, routines |
| `experience` | Skills, past events, professional background |
| `goal` | Desires, plans, aspirations |
| `guideline` | How the assistant *should* behave — rules, tone preferences, conditional instructions |

`guideline` replaces the old `is_behavioral()` / `subject == "assistant"` logic.

## Lifecycle

### 1. Real-Time Learning Trigger

After each new episode is created in `event_segmentation.rs`, a `PredictCalibrateJob` is immediately enqueued for that episode:

**Code**: `enqueue_predict_calibrate_jobs()` in `crates/worker/src/jobs/event_segmentation.rs`

Unlike the previous batch-based approach, this implements **real-time learning** where each episode is processed as soon as it's created.

### 2. Predict-Calibrate Pipeline

Implemented in `PredictCalibrateJob` (`crates/worker/src/jobs/predict_calibrate.rs`):

#### Cold Start Mode (No Existing Knowledge)

When no semantic knowledge exists yet, the system uses a specialized high-value extraction prompt:

- **Persistence Test**: Will this still be true in 6 months?
- **Specificity Test**: Does it contain concrete, searchable information?
- **Utility Test**: Can this help predict future user needs?
- **Independence Test**: Can this be understood without conversation context?

#### Normal Mode (With Existing Knowledge)

**Step 1 — PREDICT**: Generate content prediction

- Input: Episode title + existing relevant facts
- Output: Predicted episode content (free-form text)
- Function: `predict_episode()`
- Uses LLM to generate what the conversation "should" contain

**Step 2 — CALIBRATE**: Compare and extract knowledge

- Input: Predicted content + Actual messages
- Output: List of high-value knowledge statements
- Function: `predict_calibrate_extraction()`

Extracts knowledge statements that explain prediction errors:
- What couldn't be predicted (missing knowledge)
- What contradicted existing knowledge
- What refined vague existing knowledge

**Step 3 — Consolidate**: Merge with existing knowledge

- Deduplication via embedding similarity (≥0.95)
- Category inference from statement content
- Simple keyword extraction
- Source episode tracking

All statements are embedded via `embed_many()` before consolidation.

Embed input format: `"{category}: {fact} {keywords.join(" ")}"` — category prefix biases the vector toward the semantic domain.

Then, inside a single DB transaction:

| Action | Behavior |
|--------|---------|
| `new` | Embedding dedup check first (cosine sim ≥ 0.95 → merge instead). If no duplicate, insert. |
| `reinforce` | Append new source episode IDs to existing fact (no text change). |
| `update` | Invalidate old fact (`invalid_at = now()`), insert new version. |
| `invalidate` | Set `invalid_at = now()` on existing fact. |

**Hallucination guard**: `existing_fact_id` from the LLM is validated against the IDs actually presented in the prompt. Unrecognized IDs → treated as `new`.

**Deduplication constants**:
- `DEDUPE_THRESHOLD = 0.95` — cosine similarity above which facts are considered true duplicates
- `DUPLICATE_CANDIDATE_LIMIT = 5` — candidate facts checked per dedup query

#### Step 4 — Mark Episode Consolidated

The episode is marked `consolidated_at = now()`, preventing re-processing.

### 3. Temporal Validity

Facts are never hard-deleted. When a fact is contradicted:

```
Old fact: { category: "identity", fact: "User lives in Osaka", invalid_at: NULL }
     ↓  Update action
New fact: { category: "identity", fact: "User lives in Tokyo", invalid_at: NULL }
Old fact: { ..., invalid_at: "2026-02-28T..." }  ← soft-deleted
```

This preserves history and avoids information loss.

## Retrieval

### Hybrid BM25 + Vector Search

**Code**: `SemanticMemory::retrieve()` and `SemanticMemory::retrieve_by_embedding()` in `crates/core/src/memory/semantic.rs`

```sql
WITH
fulltext AS (
  SELECT id, ROW_NUMBER() OVER (ORDER BY pdb.score(id) DESC) AS r
  FROM semantic_memory
  WHERE search_text ||| $query
    AND conversation_id = $id
    AND invalid_at IS NULL
    AND ($category::text IS NULL OR category = $category)
  LIMIT 100
),
semantic AS (
  SELECT id, ROW_NUMBER() OVER (ORDER BY embedding <#> $vec) AS r
  FROM semantic_memory
  WHERE conversation_id = $id AND invalid_at IS NULL
    AND ($category::text IS NULL OR category = $category)
  LIMIT 100
),
...RRF merge...
```

BM25 runs against `search_text` (generated column = `fact || ' ' || keywords joined`), which gives keyword entities multi-hop reach.

RRF formula: `score = Σ 1.0 / (60 + rank)`

**No FSRS reranking** — semantic facts are not subject to decay.

### Category Filter

Both `retrieve()` and `retrieve_by_embedding()` accept `category: Option<&str>`. When `Some("guideline")` is passed, only guideline facts are returned. Callers (including the API) pass `None` for a full search.

### In Tool Results

`format_tool_result()` (`crates/core/src/memory/retrieval.rs`) renders semantic facts as a flat list with a `[category]` prefix:

```markdown
## Semantic Memory
- [preference] User prefers dark mode interfaces (sources: 3 conversations)
- [identity] User lives in Tokyo (sources: 1 conversation)
- [guideline] Assistant should avoid formal honorifics (sources: 2 conversations)
```

Sources count = `source_episodic_ids.len()`, indicating how many independent episodes corroborate the fact.

## Access Patterns

### Via API

Semantic facts are returned alongside episodic results from the retrieve_memory endpoint:

| Endpoint | Location |
|----------|----------|
| `POST /api/v0/retrieve_memory` | `crates/server/src/api/retrieve_memory.rs` |
| `POST /api/v0/retrieve_memory/raw` | `crates/server/src/api/retrieve_memory.rs` |
| `POST /api/v0/context_pre_retrieve` | `crates/server/src/api/retrieve_memory.rs` |

The raw endpoint returns `{ "semantic": [...], "episodic": [...] }`.

`context_pre_retrieve` returns semantic-only markdown for system prompt injection; it does **not** record a pending review.

There is **no direct write API** for semantic memory. All facts are created exclusively through the consolidation pipeline.

### Programmatic

| Operation | Location |
|-----------|----------|
| `SemanticMemory::retrieve(query, limit, conversation_id, db, category)` | `crates/core/src/memory/semantic.rs` |
| `SemanticMemory::retrieve_by_embedding(query, embedding, limit, conversation_id, db, category)` | `crates/core/src/memory/semantic.rs` |

## Migration Notes

The `search_text` generated column uses `immutable_keywords_to_text(TEXT[])`, a user-defined `IMMUTABLE` wrapper around `array_to_string`. This is required because PostgreSQL's `array_to_string` is `STABLE`, and `GENERATED ALWAYS AS … STORED` requires `IMMUTABLE` functions. The wrapper is created in migration `m20260228_01_refactor_semantic_memory`.

## Design Decisions

### Why Fact-Centric (Not SPO Triplets)?

The previous design used subject/predicate/object triples. This was replaced because:
- Natural language facts are more flexible and readable
- The LLM generates better-quality content without rigid predicate constraints
- Categories provide enough structure for filtering without locking into a taxonomy
- Deduplication works via embedding similarity regardless of structure

### Why 8 Flat Categories?

Hierarchical labels (e.g., `user/preference`, `self/guideline`) add complexity without benefit. 8 flat categories cover all relevant knowledge domains and are simple enough for an LLM to consistently assign.

### Why `keywords` Field?

BM25 on `fact` alone misses entity references. If the fact is "User's colleague Alex introduced them to Rust," the keyword `["Alex", "Rust"]` ensures BM25 can surface this fact when querying for "Alex" or "Rust" even if the fact text doesn't lead with those words.

### Why No FSRS for Semantic Memory?

Semantic facts represent persistent knowledge, not episodic events. They don't need decay modeling because:
- A fact like "user lives in Tokyo" is either true or invalidated—it doesn't "fade"
- Temporal validity is handled explicitly through `valid_at`/`invalid_at`
- FSRS complexity (stability, difficulty, review scheduling) adds overhead with no benefit for long-term knowledge

### Why Offline Consolidation?

The CLS (Complementary Learning Systems) analogy: episodic memories accumulate first, then are replayed offline to extract durable knowledge. This:
- Amortizes LLM costs across multiple episodes per consolidation call
- Allows cross-episode pattern detection (multiple mentions = stronger signal)
- Keeps the hot path (add_message → episodic creation) free of consolidation latency

### Why Soft Delete?

Hard-deleting invalidated facts would lose history. Soft deletes via `invalid_at` allow:
- Audit trail of what was once believed
- Future: temporal queries ("what did we know before X date?")
- Safe rollback if consolidation produced incorrect invalidations

## Thresholds Reference

| Constant | Value | Location |
|----------|-------|----------|
| `FLASHBULB_SURPRISE_THRESHOLD` | 0.85 | `crates/worker/src/jobs/event_segmentation.rs` |
| `DEDUPE_THRESHOLD` | 0.95 | `crates/worker/src/jobs/predict_calibrate.rs` |
| `MAX_STATEMENTS_FOR_PREDICTION` | 10 | `crates/worker/src/jobs/predict_calibrate.rs` |
| `RETRIEVAL_CANDIDATE_LIMIT` | 100 | `crates/core/src/memory/semantic.rs` |

## Relationships

```
┌──────────────────┐    creates   ┌──────────────────┐
│ EventSegmentation│─────────────▶│  EpisodicMemory  │
│     Job          │              │    (created)     │
└──────────────────┘              └──────────────────┘
         │                                 │
         │ enqueues (real-time,            │ single episode
         │ per episode)                    ▼
         │                    ┌──────────────────────┐
         └───────────────────▶│  PredictCalibrateJob │
                              │  (per episode)       │
                              └──────────────────────┘
                                         │
                              ┌──────────┴──────────┐
                              ▼                     ▼
                    ┌──────────────────┐   ┌──────────────────┐
                    │  identity /      │   │  guideline       │
                    │  preference /    │   │  (behavioral     │
                    │  interest / ...  │   │   rules)         │
                    └──────────────────┘   └──────────────────┘
                              │
                    ┌─────────┴─────────┐
                    │  retrieve_memory  │◀── Query (+ optional category filter)
                    │  (BM25+vector RRF │
                    │   on search_text, │
                    │   no FSRS)        │
                    └───────────────────┘
```

## See Also

- [Episodic Memory](episodic_memory.md) — The source material for consolidation
- [Segmentation](segmentation.md) — How episodes are created (consolidation trigger point)
- [Retrieve Memory](retrieve_memory.md) — API that surfaces both semantic and episodic results
- [FSRS](fsrs.md) — Applies to episodic memory only; semantic memory does not use FSRS
