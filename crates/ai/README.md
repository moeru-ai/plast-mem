# plastmem_ai

OpenAI-compatible AI wrapper used by the Rust crates.

## Exports

- `embed`
- `embed_many`
- `generate_text`
- `generate_object`
- `cosine_similarity`
- chat message request types re-exported from `async-openai`

## Configuration

Reads runtime configuration from `plastmem_shared::APP_ENV`.

Relevant fields:

- `OPENAI_BASE_URL`
- `OPENAI_API_KEY`
- `OPENAI_CHAT_MODEL`
- `OPENAI_CHAT_SEED`
- `OPENAI_EMBEDDING_MODEL`
- `OPENAI_REQUEST_TIMEOUT_SECONDS`

## Current usage

- segmentation LLM stages
- episode title and time-anchor generation
- memory review
- predict-calibrate extraction
- retrieval query embeddings

## Notes

- `generate_object` applies the local strict-schema normalization path before
  sending schemas to the model.
- `embed_many` is used by semantic consolidation to batch fact embeddings.
