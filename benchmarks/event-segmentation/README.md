# Event Segmentation Debug Harness

This folder holds small fixtures for iterating on worker segmentation without
running the HTTP server or the LoCoMo QA benchmark.

Run a deterministic pass:

```bash
cargo run -p plastmem_event_segmentation --features segmentation-debug --bin segmentation_debug -- --no-llm benchmarks/event-segmentation/fixtures/smoke.json
```

Run the default debug LLM path:

```bash
cargo run -p plastmem_event_segmentation --features segmentation-debug --bin segmentation_debug -- benchmarks/event-segmentation/fixtures/smoke.json
```

Run the embedding-candidate planner plus temporal fallback candidates:

```bash
cargo run -p plastmem_event_segmentation --features segmentation-debug --bin segmentation_debug -- --planner-only benchmarks/event-segmentation/fixtures/smoke.json
```

`--embedding-planner` is an explicit alias for the same embedding-driven planner path:

```bash
cargo run -p plastmem_event_segmentation --features segmentation-debug --bin segmentation_debug -- --embedding-planner benchmarks/event-segmentation/fixtures/smoke.json
```

These LLM/embedding modes call the configured local OpenAI-compatible endpoint.

Output JSON for diffing:

```bash
cargo run -p plastmem_event_segmentation --features segmentation-debug --bin segmentation_debug -- --json benchmarks/event-segmentation/fixtures/smoke.json
```

Dump a detailed LoCoMo fixture after converting it to the message JSON format:

```bash
cargo run -p plastmem_event_segmentation --features segmentation-debug --bin segmentation_debug -- --no-llm --detail /tmp/locomo-segmentation-debug/conv-47.json
```

Input format:

```json
[
  {
    "role": "John",
    "content": "I played chess today.",
    "timestamp": "2022-03-16T12:00:00Z"
  }
]
```
