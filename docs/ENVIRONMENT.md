# Environment Variables

Plast Mem uses environment variables for configuration. All variables are required.

## Required Variables

| Variable | Description | Example |
|----------|-------------|---------|
| `DATABASE_URL` | ParadeDB connection string | `postgres://user:pass@localhost:5432/plastmem` |
| `OPENAI_BASE_URL` | OpenAI-compatible API endpoint | `https://api.openai.com/v1` or `http://localhost:11434/v1` |
| `OPENAI_API_KEY` | API authentication key | `sk-...` |
| `OPENAI_CHAT_MODEL` | Model for chat completions | `gpt-5.2` |
| `OPENAI_EMBEDDING_MODEL` | Model for embeddings | `text-embedding-3-small` |

## Setup

Create a `.env` file in the project root:

```bash
DATABASE_URL=postgres://user:password@localhost:5432/plastmem
OPENAI_BASE_URL=https://api.openai.com/v1
OPENAI_API_KEY=sk-your-api-key
OPENAI_CHAT_MODEL=gpt-5.2
OPENAI_EMBEDDING_MODEL=text-embedding-3-small
```

## Self-hosted LLM Setup

For local models via Ollama or similar:

```bash
OPENAI_BASE_URL=http://localhost:11434/v1/
OPENAI_API_KEY=plastmem
OPENAI_CHAT_MODEL=gpt-oss
OPENAI_EMBEDDING_MODEL=qwen3-embedding:0.6b
```

## Client Configuration

Variables used by client applications (e.g. `examples/haru`), not the server itself:

| Variable              | Description               | Default                 |
|-----------------------|---------------------------|-------------------------|
| `PLASTMEM_BASE_URL`   | Plast Mem server endpoint | `http://localhost:3000` |

## Troubleshooting

- **Missing env var panic**: Ensure all variables are set in `.env` or environment
- **Connection errors**: Verify ParadeDB is running and accessible
- **API errors**: Check `OPENAI_BASE_URL` ends with `/v1`
