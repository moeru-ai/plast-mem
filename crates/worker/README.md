# plastmem_worker

Background job execution for Plast Mem.

## Jobs

### `EventSegmentationJob`

Files:

- `src/jobs/event_segmentation.rs`
- `crates/event_segmentation/src/batch_segmentation.rs`

Worker responsibilities:

- validate the active claim
- load claimed messages
- call the active segmentation policy
- build commit plan
- commit `episode_span`
- enqueue follow-up jobs

Policy responsibilities live in `plastmem_event_segmentation`:

- temporal rule segmentation
- primitive LLM review / split
- informative boundary review / constrained resegmentation

### `EpisodeCreationJob`

File:

- `src/jobs/episode_creation.rs`

Responsibilities:

- load the current `EpisodeSpan`
- ensure `episodic_memory` exists
- generate episode title/content
- embed and initialize FSRS state
- enqueue `PredictCalibrateJob` when needed

### `MemoryReviewJob`

File:

- `src/jobs/memory_review.rs`

Responsibilities:

- aggregate pending review items
- ask an LLM for Again/Hard/Good/Easy ratings
- update `stability`, `difficulty`, and `last_reviewed_at`

### `PredictCalibrateJob`

File:

- `src/jobs/predict_calibrate.rs`

Responsibilities:

- load related semantic facts
- run cold-start extraction or predict-calibrate extraction
- consolidate semantic actions
- mark the episode as consolidated

## Runtime model

All jobs use Apalis PostgreSQL storage. The main binary creates storages for:

- `EventSegmentationJob`
- `EpisodeCreationJob`
- `MemoryReviewJob`
- `PredictCalibrateJob`

## Notes

- `EventSegmentationJob` is lease-aware and retry-tolerant. It validates the
  current active claim before doing work.
- `EpisodeCreationJob` may re-enqueue `PredictCalibrateJob` if the episode
  already exists but has not been consolidated yet.
