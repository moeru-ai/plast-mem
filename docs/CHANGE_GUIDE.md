# Change Guide

This document provides specific instructions for common types of changes in Plast Mem.

## Quick Reference

| Change Type | Primary Crates | See Section |
|-------------|----------------|-------------|
| Add new API endpoint | server, core | [Adding an API Endpoint](#adding-an-api-endpoint) |
| Modify FSRS parameters | core, worker, shared | [FSRS Changes](#fsrs-changes) |
| Add memory field | entities, migration, core | [Schema Changes](#schema-changes) |
| Modify segmentation | core, worker | [Segmentation Changes](#segmentation-changes) |
| Add AI capability | ai, core | [AI Changes](#ai-changes) |
| Modify retrieval | core | [Retrieval Changes](#retrieval-changes) |
| Add job type | worker, core | [Adding a Job Type](#adding-a-job-type) |

## Common Change Patterns

### Adding an API Endpoint

New HTTP endpoints follow the flow: Router → Handler → Core Service.

1. **Define request/response types** in `crates/server/src/api/` or `crates/shared/src/message.rs`
2. **Add handler function** in `crates/server/src/api/<name>.rs`:
   ```rust
   #[handler]
   pub async fn handler(state: State<AppState>, req: Request) -> Response {
       // Extract inputs, call core, return response
   }
   ```
3. **Call core logic** - Don't put business logic in handlers; delegate to `plastmem_core`
4. **Register route** in `crates/server/src/server.rs` or appropriate router module
5. **Add test** in `crates/server/tests/` or as a `#[cfg(test)]` module

**Example files to reference:**
- `crates/server/src/api/add_message.rs` - Simple message ingestion
- `crates/server/src/api/retrieve_memory.rs` - Retrieval with parameters

### FSRS Changes

FSRS (Free Spaced Repetition Scheduler) parameters affect memory scheduling throughout the system.

1. **Read `docs/architecture/fsrs.md`** - Understand the algorithm first
2. **Update parameter definitions** in `crates/shared/src/fsrs.rs` if adding new parameters
3. **Modify calculation logic**:
   - `crates/core/src/memory/episodic.rs` - Retrieval and scheduling
   - `crates/core/src/memory/creation.rs` - Initial FSRS values on creation
4. **Update review logic** in `crates/worker/src/jobs/memory_review.rs`:
   - LLM evaluation mapping (Again/Hard/Good/Easy)
   - Parameter update formulas
5. **Test thoroughly** - FSRS bugs manifest as poor memory recall:
   ```bash
   cargo test -p plastmem_core fsrs
   cargo test -p plastmem_worker memory_review
   ```

**Key constraint:** FSRS state is owned by core; worker only updates via core APIs.

### Schema Changes

Database schema changes require migration and entity updates.

1. **Create migration**:
   ```bash
   cargo run --bin migration generate MIGRATION_NAME
   ```
2. **Edit migration** in `crates/migration/src/m<timestamp>_<name>.rs`:
   - Define `up` (apply) and `down` (rollback) operations
   - Use Sea-ORM migration syntax
3. **Regenerate entities** or manually update:
   - `crates/entities/src/episodic_memory.rs`
   - `crates/entities/src/message_queue.rs`
4. **Update core logic** to use new fields:
   - `crates/core/src/memory/creation.rs` - Set defaults on creation
   - `crates/core/src/memory/episodic.rs` - Use in retrieval/updates
5. **Run migration**:
   ```bash
   cargo run --bin migration up
   ```

### Segmentation Changes

Event segmentation determines when conversations become memories.

1. **Understand batch detection**: a single LLM call segments the whole window; see `docs/architecture/segmentation.md`
2. **Modify trigger logic** in `crates/core/src/message_queue/check.rs` (thresholds, fence TTL)
3. **Modify LLM segmentation** in `crates/core/src/message_queue/segmentation.rs` (prompt, output schema)
4. **Adjust job dispatch** in `crates/worker/src/jobs/event_segmentation.rs`
5. **Test with varied inputs** - segmentation quality affects memory quality

**Key constraint:** Drain happens before episode creation; a crash after drain loses messages (acceptable) but never creates duplicate episodes.

### AI Changes

The AI crate wraps LLM and embedding operations.

1. **Add provider/functionality** in `crates/ai/src/`:
   - `embed.rs` - Embedding operations
   - `generate_text.rs` - Text generation
   - `generate_object.rs` - Structured output
2. **Update public API** in `crates/ai/src/lib.rs`
3. **Handle errors** using `plastmem_shared::error`
4. **Add cost tracking** if adding new LLM calls

**Key constraint:** AI calls are expensive; always use embeddings for pre-filtering before LLM calls.

### Retrieval Changes

Memory retrieval uses hybrid ranking (BM25 + vector) with FSRS re-ranking.

1. **Understand current flow** in `crates/core/src/memory/episodic.rs`:
   - BM25 text search
   - Vector similarity
   - RRF (Reciprocal Rank Fusion)
   - FSRS retrievability weighting
2. **Modify ranking** in `crates/core/src/memory/retrieval.rs` if applicable
3. **Update review queue** - retrieval records pending reviews in `crates/core/src/message_queue/state.rs`
4. **Test retrieval quality** with representative queries

### Adding a Job Type

Background jobs handle segmentation and memory review.

1. **Define job data** in `crates/core/src/message_queue/` if it needs queue storage
2. **Create job handler** in `crates/worker/src/jobs/<name>.rs`:
   ```rust
   pub async fn run(state: AppState, job_data: JobData) -> Result<()> {
       // Job logic
   }
   ```
3. **Register in mod** at `crates/worker/src/jobs/mod.rs`
4. **Add dispatch logic** where the job is triggered (often in segmentation or API handlers)
5. **Handle failures** - jobs should be idempotent and handle retries

## Architecture Rules

### Layer Dependencies

```
plastmem_server → plastmem_core ← plastmem_worker
       ↓              ↓
   plastmem_ai    plastmem_entities
       ↓              ↓
   plastmem_shared ← plastmem_migration
```

- **Never** call DB directly from server handlers—always go through core
- **Never** spawn async tasks directly—use job queue in worker
- **Never** let core depend on server or worker (core is the inner layer)
- **Never** let shared depend on any other crate (shared is the bottom layer)

### Memory Flow Rules

1. **Creation flow**: Message → Queue → Segmentation → Episode → DB
2. **Retrieval flow**: Query → Embeddings → BM25/Vector search → FSRS rerank → Return
3. **Review flow**: Pending review → LLM evaluation → FSRS update → Mark reviewed

### Testing Rules

- Core logic should have unit tests with `#[cfg(test)]`
- API tests should use the test database
- Mock AI calls in tests—don't hit real LLM APIs
- Integration tests go in `crates/<name>/tests/`

## Commit Message Conventions

- `feat(api):` - New endpoints or API changes
- `feat(memory):` - New memory features or types
- `feat(fsrs):` - FSRS algorithm changes
- `refactor(core):` - Core logic restructuring
- `fix(segmentation):` - Boundary detection fixes
- `fix(retrieval):` - Memory retrieval fixes
- `docs:` - Documentation only changes

## Common Pitfalls

1. **Forgetting FSRS updates** - When modifying memory, check if FSRS parameters need recalculation
2. **Direct DB access** - Don't bypass core; it handles caching and consistency
3. **Missing migrations** - Entity changes need corresponding migrations
4. **Synchronous AI calls** - AI operations should be async and handle timeouts
5. **Not handling job failures** - Jobs can retry; make handlers idempotent
6. **sea-orm `cust_with_values` CAST syntax** - PostgreSQL rejects `CAST(? AS type)` in parameterized queries. Use `execute_raw` with `$1::type` cast syntax instead, or embed literals directly in the SQL string (safe for fixed-format values like UUIDs)
7. **OpenAI strict mode schema** - schemars 1.x emits `$defs`, `oneOf`, `anyOf`, and `$ref` with siblings — all rejected by strict mode. `fix_schema_for_strict` in `crates/ai/src/generate_object.rs` handles these automatically; do not bypass it
