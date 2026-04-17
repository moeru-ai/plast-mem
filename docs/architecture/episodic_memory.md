# Episodic Memory

`episodic_memory` stores committed conversation episodes plus the FSRS state
used for retrieval reranking.

## Inputs

Episodes are not created directly from message ingestion. The path is:

```text
conversation_message
  -> segmentation pipeline
  -> episode_span
  -> EpisodeCreationJob
  -> episodic_memory
```

`episode_span` is the committed range artifact. `EpisodeCreationJob` turns that
range into a stored episode record.

## Schema

Entity:

- `crates/entities/src/episodic_memory.rs`

Important fields:

| Field | Purpose |
| --- | --- |
| `id` | deterministic episode id derived from conversation + seq range |
| `conversation_id` | conversation scope |
| `messages` | preserved source messages |
| `title` | generated episode title |
| `content` | rendered episode content used for retrieval |
| `classification` | optional `low_info` / `informative` |
| `embedding` | vector embedding of episode content |
| `stability` / `difficulty` | FSRS state |
| `surprise` | currently stored but initialized to `0.0` |
| `start_at` / `end_at` | time bounds from source messages |
| `consolidated_at` | semantic consolidation completion marker |

The migration also creates a generated `search_text` column:

```text
title + " " + content
```

That is the text BM25 leg uses for episodic retrieval.

## Creation flow

Code:

- `crates/worker/src/jobs/episode_creation.rs`

Current order:

1. `try_load_current_span`
2. `try_ensure_episode_exists`
3. if missing:
   - load source messages
   - generate episode artifacts
   - embed content
   - initialize FSRS state
   - insert `episodic_memory`
4. `try_enqueue_predict_calibrate_if_needed`

## Artifact generation

Current artifact generation is split into:

### Deterministic rendering

- render transcript lines grouped by hour
- format them as `Spoken At: ...`

### Optional time anchoring

The LLM may append grounded calendar anchors to already-present time phrases.

### Title generation

The title is generated after content is finalized.

## Retrieval role

`episodic_memory` participates in hybrid retrieval:

1. BM25 on `search_text`
2. vector similarity on `embedding`
3. RRF merge
4. FSRS retrievability rerank

See [retrieve_memory](retrieve_memory.md) and [fsrs](fsrs.md).

## Consolidation role

After episode creation, informative episodes can trigger
`PredictCalibrateJob`. That job extracts or updates semantic facts and finally
sets `consolidated_at`.

## Notes

- `surprise` is still present in schema and some renderers still check it, but
  current creation writes `0.0`, so “key moment” behavior is effectively
  dormant.
- `title` and `content` are generated once on creation; there is no current
  re-render or re-summarization job.
