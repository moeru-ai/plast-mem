# FSRS Integration

## Memory State

Each memory is associated with the following state parameters:

- `stability` (S)
- `difficulty` (D)
- `last_reviewed_at`

New memories are initialized with **high retrievability but low stability**, meaning they are fresh in memory but haven't been reinforced yet.

## Reranking

Each memory retrieval first searches through a large number of similar memories, then calculates the FSRS retrievability score and re-rank them based on it.

## Review

After each memory retrieval, a `needs_review` status is pushed to the corresponding message queue.

This triggers a review task whenever a new user message is received.

The LLM-based reviewer evaluates retrieved memories and assigns a rating:

| Rating | Description |
|--------|-------------|
| **Again** | False positive - memory was retrieved but irrelevant or should be ignored |
| **Hard** | Memory provided useful context but required significant inference to apply |
| **Good** | Memory provided core information directly relevant to the query |
| ~~Easy~~ | ~~Exact match - not used~~ |

## Cleanup

TBD, it is expected that an "inactive memories" will be implemented, with permanent deletion occurring after prolonged inactivity.
