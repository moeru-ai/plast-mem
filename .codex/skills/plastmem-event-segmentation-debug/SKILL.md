---
name: plastmem-event-segmentation-debug
description: Use when iterating on Plast Mem event segmentation quality with the Rust debug harness, including running deterministic, embedding-planner, trace, detail, JSON, and LoCoMo fixture checks without starting the server or full benchmark.
---

# Plast Mem Event Segmentation Debug

Use the standalone debug bin in `plastmem_event_segmentation`:

```bash
cargo run -p plastmem_event_segmentation --features segmentation-debug --bin segmentation_debug -- [mode] [output] <messages.json>
```

Input is a JSON array of `plastmem_shared::Message`:

```json
[
  {
    "role": "John",
    "content": "I played chess today.",
    "timestamp": "2022-03-16T12:00:00Z"
  }
]
```

## Common Commands

Smoke deterministic path:

```bash
cargo run -q -p plastmem_event_segmentation --features segmentation-debug --bin segmentation_debug -- --no-llm benchmarks/event-segmentation/fixtures/smoke.json
```

Run the current embedding-candidate + LLM planner path:

```bash
cargo run -q -p plastmem_event_segmentation --features segmentation-debug --bin segmentation_debug -- --embedding-planner /tmp/locomo-segmentation-debug/conv-47.json
```

Show full message detail for manual inspection:

```bash
cargo run -q -p plastmem_event_segmentation --features segmentation-debug --bin segmentation_debug -- --embedding-planner --detail /tmp/locomo-segmentation-debug/conv-47.json
```

Show candidate and planner traces:

```bash
cargo run -q -p plastmem_event_segmentation --features segmentation-debug --bin segmentation_debug -- --embedding-planner --trace /tmp/locomo-segmentation-debug/conv-47.json
```

Output JSON for diffing:

```bash
cargo run -q -p plastmem_event_segmentation --features segmentation-debug --bin segmentation_debug -- --embedding-planner --json /tmp/locomo-segmentation-debug/conv-47.json
```

## Network Permission

`--embedding-planner`, `--planner-only`, and default mode call the configured local OpenAI-compatible LLM/embedding endpoint. If the command fails with a local socket/network sandbox error, rerun with escalated permissions.

`--no-llm` does not need network.

## Quality Checks

When comparing segmentation changes, report:

- `spans`
- `avg_messages`
- `median_messages`
- `p90_messages`
- `singleton_spans`
- `short_spans_le_3`
- `planned_boundaries`

For LoCoMo quick checks, inspect:

- `conv-47` first 37 messages: expect roughly `0..6 / 7..16 / 17..24 / 25..36`
- All 10 sample sweep: expect `singleton_spans = 0` and `short_spans_le_3 = 0`

Avoid optimizing only for span count. Prefer stable, independently retrievable event chunks without 1-3 message fragments.

## Validation Commands

After segmentation engine edits, run:

```bash
cargo test -p plastmem_event_segmentation
cargo check -p plastmem_event_segmentation --features segmentation-debug --bin segmentation_debug
cargo check
```
