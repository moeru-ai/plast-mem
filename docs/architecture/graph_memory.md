# Graph Memory

Status: not implemented in the current codebase.

There is no active graph-memory subsystem today:

- no `graph_entity` table
- no graph edge tables
- no recursive CTE retrieval route
- no graph-aware benchmark path

Current retrieval uses only:

- `semantic_memory` hybrid retrieval
- `episodic_memory` hybrid retrieval with FSRS reranking

Keep this topic as future design space only. Do not cite earlier graph-memory
docs as the current implementation.
