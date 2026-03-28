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
# interactive run
pnpm -F @plastmem/benchmark-locomo start
```

## Pipeline

```
for each selected sample:
1. Ingest    — replay the conversation turn-by-turn into plast-mem via addMessage
2. Eval      — run plast-mem QA, and optionally Full Context QA
3. Score     — compute F1 / Nemori F1 / optional LLM judge
4. Persist   — update checkpoint + aggregate output JSON
```

## Interactive Options

At startup the CLI prompts for:

- sample scope (defaults to `conv-42`, `conv-44`, `conv-48`, `conv-50`; can switch to `all` or custom selection)
- compare mode (`plast-mem only` or `plast-mem + Full Context`)
- fixed QA concurrency (`4`)
- always waits for background jobs after each sample ingest
- whether to enable LLM judge scoring

If a previous checkpoint exists in `benchmarks/locomo/results/`, the CLI first asks whether to resume that latest checkpoint. When resuming, it reuses the saved config instead of asking for fresh run options.

`PLASTMEM_BASE_URL` is read from the root `.env`. If unset, it defaults to `http://localhost:3000`.
`OPENAI_CHAT_MODEL` is read from the root `.env`. If unset, the CLI exits with an error.

## Resume / Checkpoint

- The dataset is always loaded from `benchmarks/locomo/data/locomo10.json`
- If that file is missing, the CLI exits with the expected path and a `curl` command to download it
- Each output file writes a sibling checkpoint file: `results/<run>.checkpoint.json`
- Fresh runs always write to a timestamped output path automatically
- Progress is persisted at sample-stage granularity:
  - ingest complete
  - plast-mem eval complete
  - plast-mem score complete
  - full-context eval complete
  - full-context score complete
- On restart, the CLI detects a compatible checkpoint and asks whether to resume it
- Final results and checkpoint state are stored separately

## Full Context Baseline

- `plast-mem` is always run
- `Full Context` is optional and can be enabled in the interactive prompt
- The Full Context baseline follows the LoCoMo official non-RAG style:
  - build a chronological transcript from the full conversation
  - feed that transcript directly to the answer model
  - do not apply retrieval, reranking, or query-aware chunking
- This benchmark does not truncate for context length. If the model cannot handle the transcript, the sample fails and can be resumed later.

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
  "meta": {
    "model": "...",
    "base_url": "...",
    "timestamp": "...",
    "compare_full_context": true
  },
  "variants": {
    "plastmem": {
      "stats": { "overall": { "overall": 0.42 } },
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
    },
    "full_context": {
      "stats": { "overall": { "overall": 0.37 } },
      "results": []
    }
  },
  "comparison": {
    "overall": { "score_delta": 0.05 }
  }
}
```

## Source Files

| File | Purpose |
|------|---------|
| `cli.ts` | Entry point, interactive prompts, orchestration |
| `checkpoint.ts` | Checkpoint creation, compatibility, persistence |
| `full-context.ts` | Full transcript baseline context builder |
| `ingest.ts` | Replay conversations into plast-mem |
| `retrieve.ts` | `retrieve_memory` call |
| `llm.ts` | LLM answer generation via `@xsai/generate-text` |
| `evaluation.ts` | Per-category F1 scoring |
| `stats.ts` | Aggregate stats and formatted output |
| `types.ts` | Dataset and result types |
