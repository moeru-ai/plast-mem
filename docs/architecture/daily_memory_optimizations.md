# Daily Memory Optimizations

Status: historical design note, not current implementation.

The current codebase does **not** implement:

- `memory_digest`
- observation jobs
- typed importance weights
- digest rebuild workers

Current production memory flow is still:

- segmented episodes in `episodic_memory`
- semantic facts in `semantic_memory`
- retrieval through hybrid BM25 + vector search

If work resumes on these ideas later, treat this document as background
material, not as an accurate description of running code.
