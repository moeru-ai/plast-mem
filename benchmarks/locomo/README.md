# LoCoMo Benchmark

Evaluates plast-mem's long-term memory retrieval quality using the [LoCoMo dataset](https://github.com/snap-research/locomo) — 10 annotated multi-session conversations with 5 QA categories.

## Setup

```bash
# from workspace root
pnpm i

# download dataset
curl -L https://github.com/snap-research/locomo/raw/main/data/locomo10.json --create-dirs -o benchmarks/locomo/data/locomo10.json

# configure env (root .env is loaded automatically)
PLASTMEM_BASE_URL=http://localhost:3000
OPENAI_API_KEY=...
OPENAI_BASE_URL=http://localhost:11434/v1   # Ollama or any OpenAI-compatible endpoint
OPENAI_CHAT_MODEL=qwen3:8b
```

## Usage

```bash
# full run: ingest → evaluate
pnpm -F @plastmem/benchmark-locomo start

# skip ingestion (reuse previously ingested conversations)
pnpm -F @plastmem/benchmark-locomo start -- --skip-ingest

# run specific samples only
pnpm -F @plastmem/benchmark-locomo start -- --sample-ids sample_1,sample_2

# custom input/output paths
pnpm -F @plastmem/benchmark-locomo start -- \
  --data-file ./data/locomo10.json \
  --out-file ./results/run-1.json
```

## Pipeline

```
1. Ingest    — replay each conversation turn-by-turn into plast-mem via addMessage
2. Evaluate  — for each QA pair: retrieveMemory context → LLM answer → F1 score
3. Output    — write results JSON + print per-category summary
```

## QA Categories

| # | Name | Scoring |
|---|------|---------|
| 1 | Multi-hop | Mean token-F1 over comma-split gold sub-answers |
| 2 | Single-hop | Token-F1 |
| 3 | Temporal | Token-F1 on first semicolon-delimited gold part |
| 4 | Open-domain | Token-F1 |
| 5 | Adversarial | Binary: 1 if model says "no information available" |

## Output

Results are written to `results/<timestamp>.json`:

```json
{
  "meta": { "model": "...", "base_url": "...", "timestamp": "..." },
  "stats": {
    "overall": 0.42,
    "by_category": { "1": 0.38, "2": 0.51, "3": "..." },
    "by_category_count": { "1": 120, "2": 340, "3": "..." }
  },
  "results": [
    {
      "sample_id": "...",
      "category": 2,
      "question": "...",
      "gold_answer": "...",
      "prediction": "...",
      "score": 0.8,
      "context_retrieved": "...",
      "evidence": ["..."]
    }
  ]
}
```

## Source Files

| File | Purpose |
|------|---------|
| `cli.ts` | Entry point, argument parsing, orchestration |
| `ingest.ts` | Replay conversations into plast-mem |
| `retrieve.ts` | `retrieve_memory` call |
| `llm.ts` | LLM answer generation via `@xsai/generate-text` |
| `evaluation.ts` | Per-category F1 scoring |
| `stats.ts` | Aggregate stats and formatted output |
| `types.ts` | Dataset and result types |
