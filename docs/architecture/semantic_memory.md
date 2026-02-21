# Semantic Memory

Semantic memory stores long-term facts and behavioral guidelines extracted from conversations. Where episodic memory captures *what happened*, semantic memory captures *what is known*—durable knowledge that persists and evolves across many conversations.

## Overview

```
EpisodicMemory (unconsolidated)
        │
        │  threshold: 3+ episodes, or flashbulb surprise ≥ 0.85
        ▼
SemanticConsolidationJob
        │
        │  1. Load related existing facts  (predict)
        │  2. LLM consolidation call       (calibrate)
        │  3. Process fact actions         (write)
        │  4. Mark episodes consolidated
        ▼
SemanticMemory (facts / behavioral guidelines)
```

## Schema

- **Core struct**: `crates/core/src/memory/semantic/mod.rs`
- **Consolidation pipeline**: `crates/core/src/memory/semantic/consolidation.rs`
- **Database entity**: `crates/entities/src/semantic_memory.rs`

### Field Semantics

| Field | Purpose | Mutable |
|-------|---------|---------|
| `id` | UUID v7 primary key | No |
| `conversation_id` | Isolation boundary — facts scoped per conversation | No |
| `subject` | Triplet subject (`"user"`, `"assistant"`, `"we"`) | No |
| `predicate` | Relationship type (e.g. `"likes"`, `"should"`) | No |
| `object` | Triplet object value | No |
| `fact` | Natural language sentence describing the fact | No |
| `source_episodic_ids` | UUIDs of episodes that evidence this fact (provenance) | Yes (reinforce appends) |
| `valid_at` | When this fact became valid | No |
| `invalid_at` | When this fact was superseded/contradicted; `NULL` = still active | Yes (Update/Invalidate sets) |
| `embedding` | Vector of `fact` text (used for retrieval and deduplication) | No |
| `created_at` | Record creation timestamp | No |

### Differences from Episodic Memory

Semantic memory has **no FSRS parameters** (no stability, difficulty, last_reviewed_at). It does not decay. Facts are either active (`invalid_at IS NULL`) or superseded. Instead of time-based retrievability, relevance is determined purely by hybrid search ranking.

## Two Categories of Facts

Facts are distinguished at retrieval time by their subject and predicate:

### Known Facts

Everything where `is_behavioral()` returns `false`. Captures durable information about the user, the relationship, or the world:

```
subject: "user"        predicate: "likes"      object: "dark mode"
subject: "user"        predicate: "lives_in"   object: "Tokyo"
subject: "we"          predicate: "has_routine" object: "weekly check-in on Sundays"
```

### Behavioral Guidelines

Facts where `subject == "assistant"` and predicate is `"should"`, `"should_not"`, `"should_when_*"`, or `"responds_to_*"`. Encodes communication preferences and conditional behaviors:

```
subject: "assistant"   predicate: "should_not"             object: "use formal honorifics"
subject: "assistant"   predicate: "should_when_stressed"   object: "offer calming response first"
```

**Code**: `SemanticMemory::is_behavioral()` in `crates/core/src/memory/semantic/mod.rs`

### Predicate Taxonomy

The LLM is prompted to use these predicates when applicable:

| Category | Predicates |
|----------|-----------|
| Personal | `likes`, `dislikes`, `prefers`, `lives_in`, `works_at`, `age_is`, `name_is` |
| Knowledge | `is_interested_in`, `has_experience_with`, `knows_about` |
| Relational | `communicate_in_style`, `relationship_is`, `has_shared_reference`, `has_routine` |
| Behavioral | `should`, `should_not`, `should_when_[context]`, `responds_to_[trigger]_with` |

New predicates can be created by the LLM when none of the above fit.

## Lifecycle

### 1. Consolidation Trigger

After each new episode is created, `event_segmentation.rs` checks whether to enqueue a `SemanticConsolidationJob`:

| Condition | Trigger type | `force` flag |
|-----------|-------------|-------------|
| `surprise ≥ 0.85` (flashbulb memory) | Immediate | `true` |
| `unconsolidated_count ≥ 3` | Standard threshold | `false` |

**Code**: `enqueue_semantic_consolidation()` in `crates/worker/src/jobs/event_segmentation.rs`

### 2. Consolidation Pipeline

Implemented in `process_consolidation()` (`crates/core/src/memory/semantic/consolidation.rs`):

#### Step 1 — Predict: Load Related Facts

Fetch existing active facts semantically related to the unconsolidated episodes. Uses `embed_many()` on episode summaries, then `SemanticMemory::retrieve_by_vector()` per episode, deduplicated by fact ID.

- Limit: 20 related facts presented to the LLM as context
- Only searches active facts (`invalid_at IS NULL`) in the same conversation

#### Step 2 — Calibrate: LLM Consolidation Call

Single `generate_object::<ConsolidationOutput>()` call with:

- **System**: `CONSOLIDATION_SYSTEM_PROMPT` (extraction rules + predicate taxonomy)
- **User**: Existing knowledge + episode summaries + messages

Output structure:

```rust
ConsolidationOutput {
    facts: Vec<ConsolidatedFact {
        action: FactAction,          // new | reinforce | update | invalidate
        existing_fact_id: Option<String>,
        subject: String,
        predicate: String,
        object: String,
        fact: String,                // natural language sentence
    }>
}
```

#### Step 3 — Write: Process Fact Actions

All fact sentences are batch-embedded via `embed_many()` before the transaction opens. Then, inside a single DB transaction:

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

#### Step 4 — Mark Consolidated

All episode IDs are marked `consolidated_at = now()` in the same transaction, preventing re-processing.

### 3. Temporal Validity

Facts are never hard-deleted. When a fact is contradicted:

```
Old fact: { subject: "user", predicate: "lives_in", object: "Osaka", invalid_at: NULL }
     ↓  Update action
New fact: { subject: "user", predicate: "lives_in", object: "Tokyo", invalid_at: NULL }
Old fact: { ..., invalid_at: "2026-02-21T..." }  ← soft-deleted
```

This preserves history and avoids information loss.

## Retrieval

### Hybrid BM25 + Vector Search

**Code**: `SemanticMemory::retrieve()` in `crates/core/src/memory/semantic/mod.rs`

```sql
WITH
fulltext AS (
  SELECT id, ROW_NUMBER() OVER (ORDER BY pdb.score(id) DESC) AS r
  FROM semantic_memory
  WHERE fact ||| $query AND conversation_id = $id AND invalid_at IS NULL
  LIMIT 100
),
semantic AS (
  SELECT id, ROW_NUMBER() OVER (ORDER BY embedding <#> $vec) AS r
  FROM semantic_memory
  WHERE conversation_id = $id AND invalid_at IS NULL
  LIMIT 100
),
...RRF merge...
```

RRF formula: `score = Σ 1.0 / (60 + rank)`

**No FSRS reranking** — semantic facts are not subject to decay, so there is no `× retrievability` step unlike episodic retrieval.

### In Tool Results

`format_tool_result()` (`crates/core/src/memory/retrieval.rs`) splits semantic results into two sections:

```markdown
## Known Facts
- User prefers dark mode interfaces (sources: 3 conversations)
- User lives in Tokyo (sources: 1 conversation)

## Behavioral Guidelines
- Assistant should avoid formal honorifics (sources: 2 conversations)
```

Sources count = `source_episodic_ids.len()`, indicating how many independent episodes corroborate the fact.

## Access Patterns

### Via API

Semantic facts are returned alongside episodic results from the retrieve_memory endpoint:

| Endpoint | Location |
|----------|----------|
| `POST /api/v0/retrieve_memory` | `crates/server/src/api/retrieve_memory.rs` |
| `POST /api/v0/retrieve_memory/raw` | `crates/server/src/api/retrieve_memory.rs` |

The raw endpoint returns `{ "semantic": [...], "episodic": [...] }`.

There is **no direct write API** for semantic memory. All facts are created exclusively through the consolidation pipeline.

### Programmatic

| Operation | Location |
|-----------|----------|
| `SemanticMemory::retrieve()` | `crates/core/src/memory/semantic/mod.rs` |
| `SemanticMemory::is_behavioral()` | `crates/core/src/memory/semantic/mod.rs` |
| `process_consolidation()` | `crates/core/src/memory/semantic/consolidation.rs` |

## Design Decisions

### Why SPO Triplets?

Subject-predicate-object structure enables:
- **Deduplication**: Semantically equivalent facts can be detected by embedding similarity
- **Mutation tracking**: `update` and `invalidate` actions have clear semantics
- **Behavioral separation**: `subject == "assistant"` reliably identifies behavioral guidelines

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
| `CONSOLIDATION_EPISODE_THRESHOLD` | 3 | `crates/core/src/memory/semantic/consolidation.rs` |
| `FLASHBULB_SURPRISE_THRESHOLD` | 0.85 | `crates/core/src/memory/semantic/consolidation.rs` |
| `DEDUPE_THRESHOLD` | 0.95 | `crates/core/src/memory/semantic/consolidation.rs` |
| `DUPLICATE_CANDIDATE_LIMIT` | 5 | `crates/core/src/memory/semantic/consolidation.rs` |
| `RELATED_FACTS_LIMIT` | 20 | `crates/core/src/memory/semantic/consolidation.rs` |
| `RETRIEVAL_CANDIDATE_LIMIT` | 100 | `crates/core/src/memory/semantic/mod.rs` |

## Relationships

```
┌──────────────────┐    creates   ┌──────────────────┐
│ EventSegmentation│─────────────▶│  EpisodicMemory  │
│     Job          │              │  (unconsolidated) │
└──────────────────┘              └──────────────────┘
         │                                 │
         │ enqueues (if threshold/          │ batch input
         │ flashbulb)                       ▼
         │                    ┌──────────────────────┐
         └───────────────────▶│SemanticConsolidation │
                              │       Job            │
                              └──────────────────────┘
                                         │
                              ┌──────────┴──────────┐
                              ▼                     ▼
                    ┌──────────────────┐   ┌──────────────────┐
                    │  Known Facts     │   │  Behavioral      │
                    │  (user, we,...)  │   │  Guidelines      │
                    │                 │   │  (assistant,...)  │
                    └──────────────────┘   └──────────────────┘
                              │
                    ┌─────────┴─────────┐
                    │  retrieve_memory  │◀── Query
                    │  (BM25 + vector   │
                    │   RRF, no FSRS)   │
                    └───────────────────┘
```

## See Also

- [Episodic Memory](episodic_memory.md) — The source material for consolidation
- [Segmentation](segmentation.md) — How episodes are created (consolidation trigger point)
- [Retrieve Memory](retrieve_memory.md) — API that surfaces both semantic and episodic results
- [FSRS](fsrs.md) — Applies to episodic memory only; semantic memory does not use FSRS
