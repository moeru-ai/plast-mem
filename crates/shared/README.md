# plastmem_shared

Shared types and utilities used across all crates.

## Overview

This crate contains definitions that need to be shared between the core domain logic,
server API, worker jobs, and AI processing. Keeping these in a separate crate prevents
circular dependencies.

## Key Types

### [Message](src/message.rs)

The fundamental unit of conversation with `role`, `content`, and `timestamp`.

### [AppError](src/error.rs)

Application-wide error type with HTTP status code support:

```rust
// Create a 500 error
let err = AppError::new("something went wrong");

// Create with custom status
let err = AppError::with_status(StatusCode::BAD_REQUEST, "invalid input");
```

### [AppEnv](src/env.rs)

Global application configuration via `APP_ENV`:

```rust
let database_url = &APP_ENV.database_url;
let openai_api_key = &APP_ENV.openai_api_key;
```

## Modules

- `error` - `AppError` definition
- `env` - Environment configuration (`AppEnv`, `APP_ENV`)
- `fsrs` - FSRS retrievability calculation utilities
- `message` - `Message` and `MessageRole` types

## Dependencies

This crate intentionally has minimal dependencies to maximize reusability.
Key dependencies: `serde`, `chrono`, `axum` (for error response integration).
