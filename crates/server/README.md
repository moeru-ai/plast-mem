# plastmem_server

HTTP API server for Plast Mem.

## Endpoints

### Ingestion

- `POST /api/v0/add_message`
- `POST /api/v0/import_batch_messages`

Both endpoints append messages and may enqueue `EventSegmentationJob` if core
creates a fresh segmentation claim.

### Retrieval

- `POST /api/v0/retrieve_memory`
- `POST /api/v0/retrieve_memory/raw`
- `POST /api/v0/context_pre_retrieve`

`retrieve_memory*` returns semantic and episodic results together.
`context_pre_retrieve` returns semantic-only markdown and has no pending-review
side effects.

### Recent episodic memory

- `POST /api/v0/recent_memory`
- `POST /api/v0/recent_memory/raw`

### Debug-only benchmark route

- `GET /api/v0/benchmark/job_status`

This route is compiled only in debug builds.

## OpenAPI

- `/openapi.json`
- `/openapi/`

## Notes

- The root route (`/`) returns a simple HTML page in release builds.
- The Apalis board UI is mounted under `/board` in debug builds.
- All business logic should stay in `plastmem_core`; handlers should only
  validate inputs, call core, and enqueue jobs when needed.
