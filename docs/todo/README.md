# TODO Index

This folder contains design sketches and future work notes. It grew over time; this file is the **single index** to keep priorities clear.

## Decisions & Defaults (current)

- **Flashbulb FSRS**: do **not** pin retrievability to `1.0`. Use a flashbulb-specific **soft-mix floor** multiplier:
  - `mult = floor + (1 - floor) * retrievability`
  - flashbulb `floor ≈ 0.6–0.8` (optionally scale with `surprise`)
  - non-flashbulb `floor ≈ 0.2–0.3`
  - (optional guardrail) cap flashbulb items in the final episodic top-N (e.g. max 2–3) to avoid dominance.
- **Truth vs index**: `semantic_memory` stays source-of-truth; Graph/communities/digests are derived indexes.
- **Graph route**: never always-on; enable only when multi-hop/low-confidence is detected.

## P0 (next) — high ROI, low risk

- **FSRS flashbulb soft-mix** (episodic rerank) + skip/limit review cost.
  - See: `docs/todo/flashbulb_memory.md`
  - Related: `docs/architecture/fsrs.md`
- **Strengthen `semantic_memory.keywords`** by integrating a cheap extractor into `PredictCalibrateJob` (statement + episode title/summary), to improve both BM25 and graph entity coverage.
  - See: `docs/architecture/graph_memory.md`
  - See: `docs/architecture/daily_memory_optimizations.md`

## P1 — “Mastra OM” daily UX wins (stable prefix + time anchors)

- **Stable prompt prefix cache (`memory_digest`)** rebuilt asynchronously on dirty thresholds.
  - See: `docs/architecture/daily_memory_optimizations.md`
- **Time-anchored observations** stored as derived compression inside `episodic_memory` (JSONB), generated async after episode creation.
  - See: `docs/architecture/daily_memory_optimizations.md`

## P2 — “Graphiti/Mnemis-like” multi-hop routing (LongMemEval-oriented)

- **Graph Memory tables + retrieval** (entity/community + recursive CTE + evidence paths) as a second-stage candidate generator.
  - See: `docs/architecture/graph_memory.md`
  - See: `docs/architecture/longmemeval_optimizations.md`

## P3 — semantic confidence / validity (bigger change)

- Add semantic confidence tracking / review flow.
  - See: `docs/todo/semantic_memory_confidence.md`

## Legacy notes

- Semantic extensions (older SPO-based notes; needs rethinking under category+keywords schema).
  - See: `docs/todo/semantic_memory_extension.md`
