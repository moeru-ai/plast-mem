# Plast Mem Development Context

## Project Overview

Plast Mem is an experimental llm memory layer for cyber waifu. The project is not yet stable, and limited documentation.

## How to Use This Documentation

When working on Plast Mem, follow this decision tree to navigate the codebase and make changes efficiently:

### Starting a Task

**First, understand what type of change you're making:**
- Is it a new feature? → Check docs/CHANGE_GUIDE.md for similar patterns
- Is it a refactor? → Check docs/ARCHITECTURE.md for design principles
- Is it a bug fix? → Read relevant crate README.md files

### Understanding Change Impact

**Before making changes, trace the impact:**

**Dependency flow pattern:**
```
API endpoint → Server handler → Core service → Entity/DB
     ↑              ↑              ↑
   HTTP           DTOs          Business Logic
```

**Steps:**
1. **Read the crate's README.md** to understand responsibilities
2. **Check docs/ARCHITECTURE.md** for layer dependencies
3. **Find all callers** with `grep -r "fn_name" crates/`
4. **Check trait implementations** in `plastmem_core/src/`
5. **Verify DB schema** in `plastmem_entities/src/`

### Quick Component Summary

- **plastmem**: Entry program - initializes tracing, DB, migrations, job storage, spawns worker and server
- **plastmem_core**: Core domain logic
  - `memory/episodic/mod.rs` - hybrid retrieval with FSRS re-ranking
  - `memory/episodic/creation.rs` - episode generation and FSRS initialization
  - `memory/semantic/mod.rs` - semantic fact retrieval (BM25 + vector, no FSRS)
  - `memory/semantic/consolidation.rs` - CLS consolidation pipeline (episodes → long-term facts)
  - `memory/retrieval.rs` - shared markdown formatting (`format_tool_result`, `DetailLevel`)
  - `message_queue/mod.rs` - MessageQueue struct, push/drain/get, PendingReview type
  - `message_queue/check.rs` - trigger check (count/time), fence acquisition, SegmentationCheck
  - `message_queue/segmentation.rs` - batch LLM segmentation (batch_segment, BatchSegment, SurpriseLevel)
  - `message_queue/state.rs` - fence state management, pending reviews
- **plastmem_migration**: Database table migrations
- **plastmem_entities**: Database table entities (Sea-ORM)
  - `episodic_memory.rs` - episodic memory entity
  - `semantic_memory.rs` - semantic memory entity
  - `message_queue.rs` - message queue entity
- **plastmem_ai**: AI SDK wrapper - embeddings, cosine similarity, text generation, structured output
- **plastmem_shared**: Reusable utilities (env, error)
- **plastmem_worker**: Background tasks worker
  - `event_segmentation.rs` - job dispatch, consolidation trigger
  - `memory_review.rs` - LLM-based review and FSRS update
  - `semantic_consolidation.rs` - runs CLS consolidation pipeline
- **plastmem_server**: HTTP server and API handlers
  - `api/add_message.rs` - message ingestion
  - `api/recent_memory.rs` - recent memories (raw JSON and markdown)
  - `api/retrieve_memory.rs` - semantic + episodic retrieval (raw JSON and markdown); `context_pre_retrieve` for semantic-only pre-LLM injection

## Key Runtime Flows

- **Memory creation**: `crates/server/src/api/add_message.rs` → `MessageQueue::push` (RETURNING trigger_count) → `check()` (count/time trigger + CAS fence) → `EventSegmentationJob` → `batch_segment()` (single LLM call: title + summary + surprise_level per segment) → drain + finalize → `create_episode_from_segment` (parallel, embed + FSRS init) → `EpisodicMemory` with surprise-based FSRS stability boost
- **Semantic consolidation**: after episode creation → `enqueue_semantic_consolidation` (if ≥3 unconsolidated episodes or flashbulb surprise ≥0.85) → `SemanticConsolidationJob` → load related facts → LLM consolidation call → new/reinforce/update/invalidate facts → mark episodes consolidated
- **Memory retrieval**: `crates/server/src/api/retrieve_memory.rs` → parallel: `SemanticMemory::retrieve` (BM25 + vector RRF) + `EpisodicMemory::retrieve` (BM25 + vector RRF × FSRS retrievability) → records pending review in `MessageQueue`
- **Pre-retrieval context**: `POST /api/v0/context_pre_retrieve` → `SemanticMemory::retrieve` only → returns markdown for system prompt injection; no pending review recorded
- **FSRS review update**: segmentation triggers `MemoryReviewJob` when pending reviews exist → LLM evaluates relevance (Again/Hard/Good/Easy) → FSRS parameter update in `crates/worker/src/jobs/memory_review.rs`

## Context Files

Load these additional context files when working on specific areas:

- `docs/ARCHITECTURE.md` - System-wide architecture and design principles
- `docs/ENVIRONMENT.md` - Environment variables and configuration
- `docs/CHANGE_GUIDE.md` - Step-by-step guides for common changes
- `docs/architecture/fsrs.md` - FSRS algorithm, parameters, and memory scheduling
- `docs/architecture/semantic_memory.md` - Semantic memory schema, consolidation pipeline, retrieval
- `crates/core/README.md` - Core domain logic and memory operations
- `crates/ai/README.md` - AI/LLM integration, embeddings, and structured output
- `crates/server/README.md` - HTTP API and handlers
- `crates/worker/README.md` - Background job processing

## Implementation Strategy

When implementing new features:

1. **Start with types** - Define structs/enums in `plastmem_entities` or `plastmem_core`
2. **Add core logic** - Implement business logic in `plastmem_core`
3. **Wire up API** - Add HTTP handlers in `plastmem_server`
4. **Add background jobs** - If needed, create job handlers in `plastmem_worker`

**Incremental Development**: Make small, testable changes. The codebase uses compile-time checks extensively—use `cargo check` frequently.

## Testing Conventions

- **Unit tests**: Add to `crates/<name>/src/` with `#[cfg(test)]` modules
- **Integration tests**: Add to `crates/<name>/tests/` or workspace `tests/`
- **Database tests**: Use `#[tokio::test]` with test database setup
- **AI mocking**: Tests should mock LLM calls; use fixtures for embedding vectors

## Development Notes

- **Two memory layers**: Episodic (events, FSRS-decayed) and Semantic (facts, no decay). Most features touch both.
- **FSRS applies to episodic only**: Semantic facts use temporal validity (`valid_at`/`invalid_at`) instead of decay.
- **Dual-channel detection**: Event segmentation uses a single batch LLM call with dual-channel criteria (topic shift + surprise)
- **Queue-based architecture**: Messages flow through queues; operations are often async
- **LLM costs matter**: AI calls are expensive; the system uses embeddings for first-stage retrieval
- **Consolidation is offline**: Semantic facts are extracted in background jobs, not during the hot add_message path

## File Reference

| File | Purpose |
| ---- | ------- |
| `docs/ARCHITECTURE.md` | System-wide architecture and design principles |
| `docs/architecture/fsrs.md` | FSRS algorithm and memory scheduling |
| `docs/architecture/semantic_memory.md` | Semantic memory schema, consolidation pipeline, retrieval |
| `crates/core/src/memory/episodic/mod.rs` | Episodic memory retrieval with hybrid ranking |
| `crates/core/src/memory/episodic/creation.rs` | Episode generation logic |
| `crates/core/src/memory/semantic/mod.rs` | Semantic memory retrieval |
| `crates/core/src/memory/semantic/consolidation.rs` | CLS consolidation pipeline |
| `crates/core/src/message_queue/mod.rs` | Queue push/drain/get, PendingReview type |
| `crates/core/src/message_queue/check.rs` | Trigger check, fence acquisition |
| `crates/core/src/message_queue/segmentation.rs` | Batch LLM segmentation, BatchSegment, SurpriseLevel |
| `crates/core/src/message_queue/state.rs` | Fence state management, pending reviews |
| `crates/worker/src/jobs/memory_review.rs` | LLM review and FSRS updates |
| `crates/worker/src/jobs/event_segmentation.rs` | Event segmentation job dispatch + consolidation trigger |
| `crates/worker/src/jobs/semantic_consolidation.rs` | Semantic consolidation job |
| `crates/server/src/api/add_message.rs` | Message ingestion API |
| `crates/server/src/api/retrieve_memory.rs` | Memory retrieval API (semantic + episodic); `context_pre_retrieve` for semantic-only pre-LLM injection |
| `crates/server/src/api/recent_memory.rs` | Recent episodic memories API |

## Build and Test Commands

```bash
# Basic commands
cargo build
cargo test
cargo check

# Check specific crate
cargo check -p plastmem_core
cargo test -p plastmem_core

# Run with logging
RUST_LOG=debug cargo run
```

## Remember

- The codebase follows predictable patterns. Most changes follow the same flow: API → Handler → Core → DB
- When in doubt about FSRS, check `docs/architecture/fsrs.md` and `crates/core/src/memory/episodic/mod.rs`
- When in doubt about semantic memory, check `docs/architecture/semantic_memory.md` and `crates/core/src/memory/semantic/`
- Memory operations are: creation (segmentation → episode), consolidation (episodes → semantic facts), retrieval (semantic + episodic), or review (FSRS update)
- Prefer reading existing implementations over guessing patterns
