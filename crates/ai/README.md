# plastmem_ai

AI client wrapper for OpenAI-compatible APIs.

## Overview

Provides a simplified interface for common AI operations:
- Text embeddings
- Chat completion (text generation)
- Structured output (JSON Schema)
- Cosine similarity calculation

All functions automatically handle retries, error conversion, and tracing.

## API

### Embeddings

```rust
use plastmem_ai::embed;

use sea_orm::prelude::PgVector;

let vector: PgVector = embed("text to embed").await?;
// Returns PgVector (wraps Vec<f32>) for direct database use
```

### Text Generation

```rust
use plastmem_ai::{generate_text, ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage, ChatCompletionRequestUserMessage};

let system = ChatCompletionRequestSystemMessage::from("You are a helpful assistant");
let user = ChatCompletionRequestUserMessage::from("Hello!");
let messages = vec![
    ChatCompletionRequestMessage::System(system),
    ChatCompletionRequestMessage::User(user),
];

let response = generate_text(messages).await?;
```

### Structured Output

Generate typed responses using JSON Schema:

```rust
use plastmem_ai::generate_object;
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
struct SummaryOutput {
    title: String,
    summary: String,
}

let result: SummaryOutput = generate_object(
    messages,
    "summary_generation".to_owned(),
    Some("Generate a summary".to_owned()),
).await?;
```

The schema is automatically fixed for OpenAI strict mode compatibility:
- `additionalProperties: false` and `required` added to all objects
- `$ref` sibling keys stripped (draft 7 requirement)
- `oneOf` of const strings converted to `enum`
- `anyOf: [T, null]` (`Option<T>`) unwrapped to `T`

### Cosine Similarity

```rust
use plastmem_ai::cosine_similarity;

let similarity = cosine_similarity(&vec1, &vec2)?;
// Returns f32 in range [0.0, 1.0]
```

## Configuration

Uses environment variables from `plastmem_shared::APP_ENV`:
- `OPENAI_API_KEY` - API authentication
- `OPENAI_BASE_URL` - API endpoint (optional)

## Features

- Automatic retry with exponential backoff
- Request/response tracing
- Structured error handling via `AppError`
