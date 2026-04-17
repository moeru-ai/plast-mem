# LongMemEval Optimizations

Status: historical benchmark strategy note, not current implementation.

The current repository does not implement:

- graph-route retrieval for LongMemEval
- benchmark-only rerank layers
- explicit abstention routing
- version-edge graph compaction

Current benchmark-facing logic is still the normal product pipeline plus
benchmark orchestration around it.

For active benchmark behavior, use:

- `benchmarks/locomo/README.md`
- `benchmarks/locomo/src/*`

Treat this file as future work only.
