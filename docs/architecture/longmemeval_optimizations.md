# LongMemEval Optimizations (Mastra OM + OMEGA patterns)

This doc collects tactics that are **specifically** useful for improving LongMemEval-style scores.

Important: some of these can harm day-to-day chat (verbosity, over-recall, excessive abstention).
For daily-safe practices, see `daily_memory_optimizations.md`.

---

## LongMemEval failure modes to target

1) **Multi-session bridging**: query references an entity indirectly (“their teammate”, “that project”) and needs multi-hop evidence.
2) **Temporal reasoning**: “when” questions and “current vs past” state.
3) **Knowledge updates**: contradictions, superseded preferences, evolving plans.
4) **Adversarial**: the correct behavior is to refuse/abstain when evidence is missing.

---

## Retrieval: make multi-hop explicit (Graph route + evidence paths)

If Graph Memory is implemented, use it aggressively for LongMemEval:

- community-first seeding (top-down routing)
- 2-hop expansion with controlled fanout
- convert entities → facts/episodes with provenance
- output compact evidence paths (bridge witness)

Bench defaults (more recall than daily):

- `C=3` communities, `M=80` entities/community, direct entity seeds `E=30`
- depth `2–3`, hop decay `0.6`, fanout `30` (cap hard)
- include `cooccur_fact`, `cooccur_episode`, `alias`, plus “evolved/superseded” edges if present

---

## Time anchoring: observations + “current state” views

Mastra’s key insight for temporal questions is to store time anchors as data.

### Observations as time-indexed facts

Whether stored in a separate table or inside `episodic_memory.observations`, ensure:

- `observed_at` always present (episode end date)
- `mentioned_at` present when explicitly mentioned (do not guess)

### Current-state rendering

LongMemEval rewards correct “current preference/state” answers.

Add an explicit renderer:

- “Current Facts”: `invalid_at IS NULL`
- “Known Updates”: group facts by entity/topic and show the latest version
- “Past Facts (superseded)”: only when the question is explicitly historical

To do this well, you will likely want **version edges** or “superseded_by” relationships.

---

## Typed weighting + priority (OMEGA-style)

OMEGA-like gains usually come from not treating all memories equally.

### Add / derive these signals

- `priority` / `importance` per semantic fact
- `type` buckets (even if only inferred):
  - decisions, preferences, plans/goals, stable identity, error patterns, constraints

Apply weights in scoring and in selection:

- decisions/constraints/preferences: upweight
- generic summaries: downweight
- very old episodic: downweight unless strongly connected via graph

---

## Compaction with versioning (to handle knowledge updates)

To score well on updates, “old vs new” must be explicit.

### Minimal compaction loop

1) identify near-duplicate facts with medium similarity (e.g., 0.75–0.95)
2) if compatible: merge as reinforcement (increase support)
3) if contradictory: mark old `invalid_at`, insert new version, and store a derived “superseded” edge

For LongMemEval, it’s worth adding explicit edges:

- `superseded_by` (old_fact_id → new_fact_id)
- `contradicts` (fact_id ↔ fact_id)

These edges can also participate in graph traversal for “current state” routing.

---

## Post-retrieval rerank (benchmark mode)

RRF is robust but not optimal for LongMemEval. Add a benchmark-mode rerank layer:

- entity overlap score
- path length bonus (shorter evidence paths are more reliable)
- time penalty when question implies recency/currentness
- type/priority weights
- negative feedback penalties (if you store per-item failure feedback)

Keep rerank deterministic to reduce evaluation variance.

---

## Abstention floor (adversarial correctness)

LongMemEval includes adversarial questions where the correct answer is “no info available”.

Implement an abstention signal from retrieval:

- if best evidence scores are below a threshold OR evidence paths are empty:
  - return an explicit “insufficient evidence” flag (or include a line in tool output)

Important: keep this as a **mode** so it doesn’t harm day-to-day helpfulness.

---

## Query routing and augmentation (benchmark mode)

OMEGA-style improvements often rely on routing:

- detect temporal questions (“when”, “latest”, “current”, “before/after”)
- detect multi-hop/bridging (“their”, “teammate”, “related”, “之前提到的”)
- detect counting/listing tasks (“how many”, “list all”)

Then:

- decide whether to enable graph route, increase hop depth, or switch to “current state” renderer
- optionally expand query with extracted entities/synonyms from the question (cheap, deterministic)

---

## Evaluation engineering (reduce cost, increase signal)

LongMemEval is expensive; get iteration speed by splitting metrics:

1) **retrieval-only**: evidence recall@k / answer-entity hit rate@k
2) **QA**: run answer model only when retrieval passed a minimum evidence threshold

Cache:

- embeddings (query + community + entity)
- tool outputs for identical queries (stable digest helps)

---

## References

- Mastra Observational Memory: https://mastra.ai/research/observational-memory
- OMEGA / LongMemEval leaderboard writeup: https://dev.to/singularityjason/how-i-built-a-memory-system-that-scores-954-on-longmemeval-1-on-the-leaderboard-2md3
