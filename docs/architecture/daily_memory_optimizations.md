# Daily Memory Optimizations (Mastra OM + OMEGA patterns)

This doc selects the **highest ROI** ideas from:

- Mastra “Observational Memory” (observer/reflection + stable prefix + time anchoring)
- OMEGA memory system (typed/weighted retrieval, lightweight rerank, forgetting/compaction)

…and adapts them to Plast Mem with a strict constraint: **improve day-to-day chat quality without increasing hallucination, latency, or cost**.

This is **not** a benchmark playbook. See `longmemeval_optimizations.md` for LongMemEval-specific strategies.

---

## Principles (daily-safe)

1) **Stability first**: prefer stable, cached memory prefixes over per-turn dynamic injection.
2) **Precision over recall**: fewer, higher confidence memories beat “everything relevant”.
3) **Explicit time anchors**: when time matters, store it as data, not as implicit prose.
4) **Truth layer remains semantic**: keep `semantic_memory` as source-of-truth; Graph and caches are derived.
5) **Cost-containment**: do expensive summarization only when something is dirty/important.

---

## Core change: a stable, cacheable memory prefix

Mastra’s biggest day-to-day win is a **stable prompt prefix** that can be cached and reused.

### Add `memory_digest` (single-row per `conversation_id`)

Store a compact, stable markdown prefix that is used by default for system prompt injection.

Suggested fields:

- `conversation_id` (PK)
- `version` (monotonic)
- `markdown` (stable format)
- `token_count`
- `updated_at`
- `dirty` (boolean)
- `source_episode_ids` (optional provenance)

### Update flow (async + threshold-based)

Trigger digest rebuild only when needed:

- new episode created (and observations/reflections updated)
- semantic facts changed (insert/update/invalidate)
- graph communities changed (if enabled)
- token budget exceeded, or N changes accumulated

Keep rebuild as a background worker job.

### Digest structure (stable, low-noise)

Suggested fixed sections (order must be stable):

1) `## Current Facts` (active `semantic_memory` only; top-N by importance; include guidelines)
2) `## Long-Term Preferences` (subset of facts, type-filtered)
3) `## Recent Key Events` (flashbulb or high-surprise episodes; short)
4) `## Time Anchors` (explicit date facts + known schedule patterns)

Avoid “open domain” dumping. The goal is to make the assistant feel consistent, not encyclopedic.

---

## Observations: integrate into episodic (time-anchored compression)

Mastra’s “observations” can be implemented as a derived view stored inside `episodic_memory`, which is day-to-day friendly and keeps schema minimal.

### Add `episodic_memory.observations` (JSONB)

Each observation should be short and **time-anchored**.

Suggested shape:

```json
{
  "text": "User is learning Rust for a low-latency trading system.",
  "observed_at": "2023-08-14",
  "mentioned_at": null,
  "entities": ["rust", "trading system"],
  "importance": 0.7
}
```

### Observation generation (cheap, async)

After episode creation, enqueue `ObservationJob` using a cheaper model:

- input: episode title, summary, messages (with timestamps)
- output: 5–15 observations, each with explicit `observed_at` and inferred `mentioned_at` when possible

Daily-safe constraints:

- if dates are not explicit, leave `mentioned_at = null` (do not guess)
- observations must be non-sensitive and non-speculative (fact-like)
- keep them short enough to be merged into digest without bloating

---

## Typed importance (OMEGA-style) without overfitting

OMEGA uses type/priority to improve ranking and compaction. For daily use, keep it simple.

### Add one lightweight signal to semantic facts

Add one of:

- `importance REAL NOT NULL DEFAULT 0.5`, or
- `support_count INT NOT NULL DEFAULT 1` (increment when reinforced)

Use it to:

- select which facts appear in `memory_digest`
- suppress low-importance memories from tool output by default
- weight graph edges/community membership (derived)

This reduces “memory spam” in everyday chat.

---

## Lightweight rerank (precision guardrails)

OMEGA-like rerank can help, but daily use must avoid complexity and latency.

Apply a small Rust-side rerank after retrieval with cheap features:

- keyword overlap count (normalized tokens)
- entity overlap (from keywords/entity graph)
- recency prior for episodic (already captured by FSRS, but can be a tiebreaker)
- conflict penalty: facts with known updates/invalidations get downweighted

Daily-safe rule: if top scores are low, prefer showing **fewer** memories rather than more.

---

## Safe forgetting / compaction (daily version)

The daily-safe variant is conservative:

- only compact *derived* artifacts (`memory_digest`, community summaries)
- never delete `semantic_memory` facts; rely on `invalid_at` and versioning
- dedupe exact duplicates + near-duplicates above a high threshold

This avoids “memory drift” where the assistant confidently repeats outdated compressed summaries.

---

## Recommended default behavior (day-to-day)

1) System prompt injection uses **only `memory_digest`** by default.
2) Tool retrieval (`retrieve_memory`) is used only when:
   - user asks explicitly to recall specifics, or
   - the assistant detects low confidence.
3) Graph route (if enabled) is **second-stage** and **triggered**, not always-on.

---

## Implementation map (Plast Mem)

Good integration points:

- Episode creation: `crates/worker/src/jobs/event_segmentation.rs` → enqueue `ObservationJob`
- Semantic updates: `crates/worker/src/jobs/predict_calibrate.rs` → mark digest dirty / rebuild
- Digest builder: new worker job (parallel to existing jobs)
- Injection endpoint: add `POST /api/v0/context_prefix` (or extend `context_pre_retrieve`)

---

## References

- Mastra Observational Memory: https://mastra.ai/research/observational-memory
- OMEGA / LongMemEval leaderboard writeup: https://dev.to/singularityjason/how-i-built-a-memory-system-that-scores-954-on-longmemeval-1-on-the-leaderboard-2md3
