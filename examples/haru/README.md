# Haru

A terminal-dwelling companion who remembers everything — or at least, tries her best.

## About

Haru is a **Heuristic Attention and Retrieval Unit** (but don't call her that unless asked). She lives in your terminal, powered by the Plast Mem memory layer, and she's genuinely, almost painfully curious about you.

She's not a perfect archive. She forgets in patterns — the things you return to stay bright and eager, the things you leave behind fade into the distance. When she remembers, it means something mattered enough to keep. When she forgets, she'll tell you honestly.

### Character

- **Curious by default**: Half-finished stories haunt her like doors left open
- **Earnestly persistent**: She asks "why?" and then "and then?" and then "what happened next?"
- **Multilingual**: Follows your lead — English, 中文, 日本語 — she matches your language naturally
- **Tech-aware**: Knows her Rust from her Go, asks what you're building before she judges
- **Memory-fragile**: Surprised by her own forgetting, grateful when you correct her

## Quick Start

> **Note**: This package is not published. You need to run the full Plast Mem stack.

### Prerequisites

- Plast Mem server running (see main project)
- OpenAI API key or compatible endpoint

### Environment

The following variables are read from the root `.env`:

|Variable|Description|
|---|---|
|`OPENAI_BASE_URL`|OpenAI-compatible API endpoint|
|`OPENAI_API_KEY`|API key|
|`OPENAI_CHAT_MODEL`|Chat model name|
|`PLASTMEM_BASE_URL`|Plast Mem server URL (default: `http://localhost:3000`)|

### Run

```bash
pnpm install
pnpm dev
```

The app will connect to your local Plast Mem instance and start chatting.

## How It Works

- **Persistent identity**: conversation ID is stored at `~/.config/haru/id` and reused across sessions
- **Session start**: fetches `recent_memory` and injects it into the system prompt
- **Each turn**: auto-calls `add_message` for both user and assistant messages
- **Memory retrieval**: `retrieve_memory` is exposed as an LLM tool — Haru calls it when she needs to look something up

## Design Philosophy

Haru was designed to test Plast Mem's memory layer, but she's also an exploration in:

- **Human-AI relationship**: Not a tool, someone who stays in the loop
- **Forgetting as feature**: Memory that fades and strengthens, like real memory
- **Curiosity-driven interaction**: She pursues because she cares about knowing *you*
