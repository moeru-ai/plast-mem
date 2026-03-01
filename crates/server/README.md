# plastmem_server

HTTP API server for Plast Mem.

## Overview

Axum-based HTTP server providing REST API endpoints for:

- Adding messages to conversations
- Retrieving semantic facts and episodic memories
- Pre-retrieval semantic context injection for system prompts

Includes OpenAPI documentation served via Scalar UI.

## API Endpoints

### POST /api/v0/add_message

Add a message to a conversation queue (triggers background segmentation):

```json
{
  "conversation_id": "550e8400-e29b-41d4-a716-446655440001",
  "message": {
    "role": "user",
    "content": "Hello, how are you?"
  }
}
```

### POST /api/v0/retrieve_memory

Search memories with hybrid retrieval. Returns Markdown-formatted results optimized for LLM tool use. Records a pending FSRS review.

```json
{
  "query": "what did we discuss about Rust",
  "conversation_id": "550e8400-e29b-41d4-a716-446655440001",
  "episodic_limit": 5,
  "semantic_limit": 20,
  "detail": "auto",
  "category": null
}
```

Response:

```markdown
## Semantic Memory
- [experience] User has been doing Python for 5 years (sources: 2 conversations)
- [guideline] Assistant should use practical examples (sources: 1 conversation)

## Episodic Memories

### Career switch to Rust [rank: 1, score: 0.92, key moment]
**When:** 2 days ago
**Summary:** User switching careers from Python to Rust...

**Details:**
- user: "I've been doing Python for 5 years..."
```

### POST /api/v0/retrieve_memory/raw

Same search, returns JSON with `{ "semantic": [...], "episodic": [...] }`. Records a pending FSRS review.

### POST /api/v0/context_pre_retrieve

Semantic-only retrieval for system prompt injection. Does **not** record a pending review.

```json
{
  "query": "User is asking about career decisions",
  "conversation_id": "550e8400-e29b-41d4-a716-446655440001",
  "semantic_limit": 20,
  "detail": "auto",
  "category": null
}
```

Returns Markdown of semantic facts only.

### POST /api/v0/recent_memory

Recent episodic memories, optionally filtered by recency. Returns Markdown.

### POST /api/v0/recent_memory/raw

Recent episodic memories as JSON array.

## Running the Server

```rust
use plastmem_server::server;

server(db, segment_job_storage).await?;
```

## OpenAPI Documentation

Available at `/openapi/` when server is running:

- Interactive docs (Scalar UI)
- Raw spec at `/openapi.json`

## Request Types

### AddMessage

```rust
pub struct AddMessage {
    pub conversation_id: Uuid,
    pub message: AddMessageMessage,
}

pub struct AddMessageMessage {
    pub role: MessageRole,
    pub content: String,
    pub timestamp: Option<DateTime<Utc>>,
}
```

### RetrieveMemory

```rust
pub struct RetrieveMemory {
    pub conversation_id: Uuid,
    pub query: String,
    pub episodic_limit: u64,        // default: 5
    pub semantic_limit: u64,        // default: 20
    pub detail: DetailLevel,        // default: Auto
    pub category: Option<String>,   // optional semantic category filter
}
```

### ContextPreRetrieve

```rust
pub struct ContextPreRetrieve {
    pub conversation_id: Uuid,
    pub query: String,
    pub semantic_limit: u64,        // default: 20
    pub detail: DetailLevel,        // default: Auto
    pub category: Option<String>,   // optional semantic category filter
}
```

`DetailLevel` controls whether full message details are included in episodic results:

- `None` - Never include details
- `Low` - Only rank 1 with high surprise
- `Auto` - Ranks 1-2 with surprise >= 0.7 (default)
- `High` - Always include details

## Architecture

- `api/` - HTTP handlers and request/response types
- `utils/` - Server utilities (AppState, shutdown handling)
- `server.rs` - Axum server setup

All endpoints return `Result<Json<T>, AppError>` with proper status codes.
