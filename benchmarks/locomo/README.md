# LoCoMo Benchmark

LoCoMo benchmark runner for the current Plast Mem stack.

## Package

- workspace package: `@plastmem/benchmark-locomo`
- entry script: `pnpm -F @plastmem/benchmark-locomo start`

## Setup

```bash
pnpm install
curl -L https://github.com/snap-research/locomo/raw/main/data/locomo10.json --create-dirs -o benchmarks/locomo/data/locomo10.json
```

The benchmark reads the root `.env` with `loadEnvFile(...)`.

Useful variables:

- `PLASTMEM_BASE_URL`
- `OPENAI_API_KEY`
- `OPENAI_BASE_URL`
- `OPENAI_CHAT_MODEL`
- `OPENAI_CHAT_SEED`

## Runtime shape

For each sample the runner:

1. replays the source conversation into Plast Mem
2. waits for background jobs to finish
3. runs plast-mem QA
4. optionally runs the full-context baseline
5. scores and persists checkpoints/results

## Server dependency

The benchmark expects a running Plast Mem server. It polls the debug-only
endpoint:

- `GET /api/v0/benchmark/job_status`

to decide when ingestion has fully settled.

## Related helpers

Useful scripts in this package:

- `src/cli.ts`: interactive runner
- `src/checkpoint.ts`: resume and compatibility checks
- `src/ingest.ts`: sample replay
- `src/retrieve.ts`: benchmark retrieval calls
- `src/export-memories.ts`: export episodic/semantic memories for one conversation

## Notes

- this workspace uses `pnpm`, not `npm`
- TypeScript linting follows the repo-wide CLI conventions in `docs/TYPESCRIPT.md`
