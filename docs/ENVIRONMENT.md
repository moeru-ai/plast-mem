# Environment Variables

Runtime configuration is loaded from `dotenvy` through
`plastmem_shared::APP_ENV`.

## Required server variables

| Variable | Description |
| --- | --- |
| `DATABASE_URL` | PostgreSQL connection string |
| `OPENAI_BASE_URL` | OpenAI-compatible base URL; trailing slash is trimmed |
| `OPENAI_API_KEY` | API key for chat and embedding calls |
| `OPENAI_CHAT_MODEL` | model name used by text and structured generation |
| `OPENAI_EMBEDDING_MODEL` | model name used by embeddings |

## Optional server variables

| Variable | Default | Description |
| --- | --- | --- |
| `OPENAI_CHAT_SEED` | unset | optional deterministic seed passed to chat generation |
| `OPENAI_REQUEST_TIMEOUT_SECONDS` | `60` | request timeout for AI calls |
| `ENABLE_FSRS_REVIEW` | `true` | enables pending review recording and `MemoryReviewJob` enqueueing |
| `PREDICT_CALIBRATE_CONCURRENCY` | `4` | worker-side concurrency budget for predict-calibrate |

## Example `.env`

```bash
DATABASE_URL=postgres://user:password@localhost:5432/plastmem
OPENAI_BASE_URL=https://api.openai.com/v1
OPENAI_API_KEY=sk-your-api-key
OPENAI_CHAT_MODEL=gpt-5.2
OPENAI_EMBEDDING_MODEL=text-embedding-3-small
OPENAI_CHAT_SEED=42
OPENAI_REQUEST_TIMEOUT_SECONDS=60
ENABLE_FSRS_REVIEW=true
PREDICT_CALIBRATE_CONCURRENCY=4
```

## Client-side variables

These are read by examples or benchmarks, not by `APP_ENV`:

| Variable | Default | Used by |
| --- | --- | --- |
| `PLASTMEM_BASE_URL` | `http://localhost:3000` | `examples/haru`, `benchmarks/locomo` helper scripts |
| `HARU_CONVERSATION_ID` | unset | `examples/haru` persistent conversation selection |

## Notes

- The HTTP server listens on `0.0.0.0:3000`; there is no env var for server port yet.
- Benchmarks often call `loadEnvFile()` manually from the workspace root. That is
  separate from `APP_ENV`.
- This repo expects `pnpm` for the TypeScript workspace.
