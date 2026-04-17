# Haru

Terminal chat client built on top of the current Plast Mem HTTP API.

## Package

- workspace package: `@plastmem/haru`
- run with `pnpm dev`

## What it does

Current Haru integration uses:

- `recentMemoryRaw` at session start / conversation switch
- `contextPreRetrieve` before each assistant generation
- `retrieveMemory` as an explicit tool
- `addMessage` for both user and assistant turns

This means Haru uses:

- semantic pre-retrieval for prompt shaping
- explicit tool-time retrieval for long-tail lookups
- server-side persistence for every turn

## Environment

Haru reads from the root `.env` plus local persisted conversation state.

Useful variables:

- `OPENAI_BASE_URL`
- `OPENAI_API_KEY`
- `OPENAI_CHAT_MODEL`
- `PLASTMEM_BASE_URL`
- `HARU_CONVERSATION_ID`

## Run

```bash
pnpm install
pnpm dev
```

## Notes

- Haru is an example client, not the source of truth for memory logic.
- The authoritative behavior lives in the Rust server/core/worker crates.
