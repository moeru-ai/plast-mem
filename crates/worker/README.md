# plastmem_worker

Background job worker for Plast Mem.

## Overview

Runs three background job processors:

1. **Event Segmentation** - Runs the segmentation engine and creates episodic memories
2. **Memory Review** - Evaluates retrieved memories and updates FSRS parameters
3. **Predict-Calibrate** - Real-time knowledge learning from episodic memories

Uses [Apalis](https://github.com/apalis-rs/apalis) for job queue management with PostgreSQL storage.

## Job Types

### [EventSegmentationJob](src/jobs/event_segmentation.rs)

Triggered when the conversation `segmentation_state` has enough pending messages or sees EOF.

Processing flow:

1. Load the claimed `conversation_message` range from `segmentation_state`
2. Call `plastmem_event_segmentation` to produce closed spans
3. Persist `episode_span` rows and provisional `episodic_memory` projections
4. Advance `segmentation_state`
5. Enqueue pending review and Predict-Calibrate jobs

The segmentation algorithm lives in the `plastmem_event_segmentation` crate. It uses temporal-gap fallback plus embedding-selected candidate hints and a window-level LLM planner; this worker module only owns job orchestration, DB persistence, and downstream enqueueing.

### [MemoryReviewJob](src/jobs/memory_review.rs)

Triggered after retrieval to evaluate memory relevance.

Processing flow:

1. Aggregate pending reviews (deduplicate memory IDs)
2. Call LLM to evaluate relevance (Again/Hard/Good/Easy)
3. Update FSRS parameters based on rating

### [PredictCalibrateJob](src/jobs/predict_calibrate.rs)

Triggered immediately after each episode is created for real-time learning.

Processing flow:

1. Load the newly created episode
2. Check if episode is already consolidated (skip if yes)
3. Load related existing semantic facts via hybrid search
4. If no existing knowledge → cold start extraction
5. Otherwise:
   - PREDICT: Generate content prediction from relevant facts (guidelines prioritized)
   - CALIBRATE: Compare prediction with actual messages, extract knowledge from gaps
6. Consolidate extracted facts (deduplicate, categorize, embed)
7. Mark episode as consolidated

## Usage

Start the worker:

```rust
use plastmem_worker::worker;

worker(db, segmentation_storage, review_storage, semantic_storage).await?;
```

This runs indefinitely until SIGINT (Ctrl+C) is received.

## Worker Configuration

Each worker has:

- **Name**: "event-segmentation", "memory-review", or "predict-calibrate"
- **Tracing**: Enabled via `enable_tracing()`
- **Shutdown timeout**: 5 seconds

## Error Handling

Jobs use `WorkerError` as a boundary type to satisfy Apalis constraints.
Internal errors are `AppError`, converted at the job boundary.

## Module Structure

- `jobs/mod.rs` - Job definitions and error types
- `jobs/event_segmentation.rs` - Event segmentation job orchestration and persistence
- `../event-segmentation` - Segmentation engine and debug harness support
- `jobs/memory_review.rs` - Review job implementation
- `jobs/predict_calibrate.rs` - Predict-Calibrate Learning job implementation
- `lib.rs` - Worker registration and monitor setup
