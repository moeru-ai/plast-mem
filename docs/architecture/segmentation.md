# Segmentation

Current segmentation is a stateful worker pipeline built on:

- `conversation_message`
- `segmentation_state`
- `episode_span`
- `plastmem_event_segmentation::batch_segmentation`

It no longer uses the old `message_queue` table or the old single-call batch
segmenter.

## Flow

```text
append_message / append_batch_messages
  -> update segmentation_state
  -> try_claim_segmentation_job
  -> enqueue EventSegmentationJob

EventSegmentationJob
  -> validate active claim
  -> load claimed messages
  -> temporal_rule_segmenter
  -> primitive_review_llm_segmenter
  -> temporal_boundary_review_llm_segmenter
  -> build_commit_plan
  -> commit_segmentation_job
  -> enqueue EpisodeCreationJob for finalized spans
  -> optionally enqueue MemoryReviewJob
  -> optionally enqueue follow-up EventSegmentationJob
```

## State model

`segmentation_state` tracks:

- `last_message_seq`
- `eof_identified`
- `next_segment_start_seq`
- current active claim:
  - `active_segment_start_seq`
  - `active_segment_end_seq`
  - `active_since`

Core owns claim creation, stale recovery, commit, and abort.

Worker still re-validates the claim before running, because an already-enqueued
job may be stale by the time it is consumed.

## Active policy stages

### 1. Temporal rule segmentation

Code:

- `crates/event_segmentation/src/batch_segmentation.rs`

Behavior:

- gap >= 30 minutes -> soft boundary candidate
- gap > 3 hours -> hard boundary candidate

This produces `RuleSegOutput` and bucket ranges.

### 2. Primitive review

For each bucket:

- if bucket length <= 4 messages: run classification LLM and decide
  `low_info` vs `informative`
- if bucket length <= 20 messages and > 4: keep as one informative segment
- if bucket length > 20: run split LLM, then classify small child segments

Outputs:

- `ReviewedSegment`
- `ReviewedBoundary`

### 3. Informative boundary review

The pipeline then merges adjacent informative segments across `RuleSoft`
boundaries while the group stays below the second-stage threshold.

- group length threshold: 30 messages
- only soft boundaries are eligible
- hard boundaries stay locked

Eligible groups are passed to constrained resegmentation.

## Segment classes

Current final classification is binary:

- `low_info`
- `informative`

Worker maps this to `EpisodeClassification` before writing `episode_span`.

## Commit semantics

`build_commit_plan` runs in the worker pipeline.

Rules:

- if `eof_identified = true`: finalize all reviewed segments
- if not EOF and there is only one reviewed segment: finalize nothing, carry it
  forward
- if not EOF and there are multiple reviewed segments: finalize all except the
  last one, and carry the last segment forward

This keeps the latest unresolved conversational tail available for future
messages.

## Side effects after commit

After `commit_segmentation_job`:

- finalized spans become `EpisodeCreationJob`s
- pending review items may become one `MemoryReviewJob`
- if more unsegmented work is now claimable, a new `EventSegmentationJob` is
  enqueued

## Current code split

- policy: `crates/event_segmentation/src/batch_segmentation.rs`
- runtime orchestration: `crates/worker/src/jobs/event_segmentation.rs`
- claim and commit state: `crates/core/src/message_ingest.rs`,
  `crates/core/src/segmentation_state.rs`
