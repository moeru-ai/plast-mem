---
name: locomo-segmentation-debugger
description: Run and inspect the LoCoMo event segmentation debugger in `crates/event_segmentation/examples/locomo_segmenter.rs`. Use when debugging `EventSegmenter`, checking segment boundaries on LoCoMo samples, inspecting the emitted event-segment JSON array, slicing or rewriting derived JSON through shell pipes or a REPL, focusing on `conv-47`, or sweeping every sample to judge overall behavior instead of a single conversation.
---

# LoCoMo Segmentation Debugger

Use `crates/event_segmentation/examples/locomo_segmenter.rs` as the primary debugger for segmentation work. It emits exactly one `Vec<EventSegment>` JSON document on `stdout`; warnings and errors go to `stderr`.

## Environment Note

`locomo_segmenter` calls the embedding backend through `plastmem_ai::embed_many`. In Codex's sandbox, requests to the local provider (for example `http://localhost:11434`) may fail even when the service is healthy on the host. If the run errors at the embedding request step, re-run with escalated permissions instead of assuming the segmenter itself is broken.

## Workflow

1. Start with a targeted run on `conv-47` when iterating on segmentation logic. It is a strong long-context sample and is usually worth checking first.
2. If the question is about event flattening rather than boundary decisions, add `--print-events` and inspect `stderr`.
3. If the question is about JSON shape or a suspicious segment, save `stdout` to a file and inspect or transform it with the commands in [references/commands.md](references/commands.md).
4. If a change looks good on `conv-47`, run every sample before concluding anything about quality. Single-sample wins are not enough.

## Rules

- Treat `stdout` as machine-readable output only. Do not parse diagnostics from it.
- Treat `stderr` as human diagnostics only. Expect sample metadata, flattened events, warnings, and errors there.
- If embedding requests fail inside the sandbox, request escalation and retry before judging segmentation quality.
- Prefer `conv-47` for focused debugging.
- Prefer all samples for regressions, distribution shifts, or “is this actually better?” questions.
- Use pipe or REPL edits only on derived JSON files. The debugger itself reads LoCoMo input and produces fresh segment JSON; it does not consume edited segment JSON back in.

## Quick Use

Use these common entry points:

```bash
cargo run -q -p plastmem_event_segmentation --example locomo_segmenter -- --sample-id conv-47 > /tmp/conv-47.segments.json
```

```bash
cargo run -q -p plastmem_event_segmentation --example locomo_segmenter -- --sample-id conv-47 --print-events > /tmp/conv-47.segments.json
```

Then load [references/commands.md](references/commands.md) for:

- single-sample commands
- all-sample sweeps
- `node` pipe filters for partial JSON inspection
- `node` REPL snippets for ad hoc mutation or extraction
- pretty-print helpers when `jq` is unavailable
