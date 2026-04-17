use apalis::prelude::{Data, TaskSink};
use apalis_postgres::PostgresStorage;
use chrono::Utc;
use plastmem_core::{
  EpisodeSpan, SegmentJobState, SegmentationJobClaim, abort_segmentation_job,
  commit_segmentation_job, get_claim_messages, get_segmentation_state, take_pending_review_items,
  try_claim_segmentation_job,
};
use plastmem_entities::EpisodeClassification;
use plastmem_event_segmentation::{
  ReviewedSegment, SegmentClassification, primitive_review_llm_segmenter,
  temporal_boundary_review_llm_segmenter, temporal_rule_segmenter,
};
use plastmem_shared::{APP_ENV, AppError, Message};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{EpisodeCreationJob, MemoryReviewJob};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventSegmentationJob {
  pub conversation_id: Uuid,
  pub active_segment_start_seq: i64,
  pub active_segment_end_seq: i64,
}

impl EventSegmentationJob {
  pub const fn to_claim(&self) -> SegmentationJobClaim {
    SegmentationJobClaim {
      conversation_id: self.conversation_id,
      active_segment_start_seq: self.active_segment_start_seq,
      active_segment_end_seq: self.active_segment_end_seq,
    }
  }

  pub const fn from_claim(claim: SegmentationJobClaim) -> Self {
    Self {
      conversation_id: claim.conversation_id,
      active_segment_start_seq: claim.active_segment_start_seq,
      active_segment_end_seq: claim.active_segment_end_seq,
    }
  }
}

#[derive(Debug, Clone)]
struct CommitPlan {
  finalized_segments: Vec<ReviewedSegment>,
  next_segment_start_seq: i64,
}

struct SegmentationContext<'a> {
  db: &'a sea_orm::DatabaseConnection,
  segmentation_storage: &'a PostgresStorage<EventSegmentationJob>,
  episode_creation_storage: &'a PostgresStorage<EpisodeCreationJob>,
  review_storage: &'a PostgresStorage<MemoryReviewJob>,
}

impl<'a> SegmentationContext<'a> {
  const fn new(
    db: &'a sea_orm::DatabaseConnection,
    segmentation_storage: &'a PostgresStorage<EventSegmentationJob>,
    episode_creation_storage: &'a PostgresStorage<EpisodeCreationJob>,
    review_storage: &'a PostgresStorage<MemoryReviewJob>,
  ) -> Self {
    Self {
      db,
      segmentation_storage,
      episode_creation_storage,
      review_storage,
    }
  }
}

// ──────────────────────────────────────────────────
// Entry
// ──────────────────────────────────────────────────

pub async fn process_event_segmentation(
  job: EventSegmentationJob,
  db: Data<sea_orm::DatabaseConnection>,
  segmentation_storage: Data<PostgresStorage<EventSegmentationJob>>,
  episode_creation_storage: Data<PostgresStorage<EpisodeCreationJob>>,
  review_storage: Data<PostgresStorage<MemoryReviewJob>>,
) -> Result<(), AppError> {
  let db = &*db;
  let ctx = SegmentationContext::new(
    db,
    &*segmentation_storage,
    &*episode_creation_storage,
    &*review_storage,
  );
  let claim = job.to_claim();

  let Some(eof_identified) = validate_claim_or_recover(&claim, &ctx).await? else {
    return Ok(());
  };

  let claimed_messages = get_claim_messages(&claim, ctx.db).await?;
  let result = process_claimed_segment_range(&claim, eof_identified, &claimed_messages, &ctx).await;

  if let Err(err) = result {
    if let Err(abort_err) = abort_segmentation_job(&claim, ctx.db).await {
      tracing::error!(
        conversation_id = %claim.conversation_id,
        active_segment_start_seq = claim.active_segment_start_seq,
        active_segment_end_seq = claim.active_segment_end_seq,
        error = %abort_err,
        "Failed to abort segmentation job after processing error"
      );
      return Err(err);
    }

    match try_claim_and_enqueue_segmentation_job(
      claim.conversation_id,
      "processing_error_recovery",
      ctx.db,
      ctx.segmentation_storage,
    )
    .await
    {
      Ok(true) => {
        tracing::warn!(
          conversation_id = %claim.conversation_id,
          active_segment_start_seq = claim.active_segment_start_seq,
          active_segment_end_seq = claim.active_segment_end_seq,
          error = %err,
          "Segmentation job failed; aborted claim and enqueued a fresh retry"
        );
        return Ok(());
      }
      Ok(false) => {
        tracing::warn!(
          conversation_id = %claim.conversation_id,
          active_segment_start_seq = claim.active_segment_start_seq,
          active_segment_end_seq = claim.active_segment_end_seq,
          error = %err,
          "Segmentation job failed; aborted claim but no fresh retry was enqueued"
        );
      }
      Err(recovery_err) => {
        tracing::error!(
          conversation_id = %claim.conversation_id,
          active_segment_start_seq = claim.active_segment_start_seq,
          active_segment_end_seq = claim.active_segment_end_seq,
          processing_error = %err,
          recovery_error = %recovery_err,
          "Segmentation job failed and recovery re-trigger also failed"
        );
      }
    }
    return Err(err);
  }

  Ok(())
}

// ──────────────────────────────────────────────────
// Main flow
// ──────────────────────────────────────────────────

// Core recovers stale leases before creating a fresh claim. This worker-side
// check guards already-enqueued jobs against races, retries, and duplicate
// deliveries after the active lease has moved on.
async fn validate_claim_or_recover(
  claim: &SegmentationJobClaim,
  ctx: &SegmentationContext<'_>,
) -> Result<Option<bool>, AppError> {
  let state = get_segmentation_state(claim.conversation_id, ctx.db).await?;
  match state.job_state {
    SegmentJobState::Active {
      active_segment_start_seq,
      active_segment_end_seq,
      ..
    } if active_segment_start_seq == claim.active_segment_start_seq
      && active_segment_end_seq == claim.active_segment_end_seq =>
    {
      Ok(Some(state.eof_identified))
    }
    _ => {
      let re_enqueued = try_claim_and_enqueue_segmentation_job(
        claim.conversation_id,
        "stale_job_recovery",
        ctx.db,
        ctx.segmentation_storage,
      )
      .await?;
      tracing::debug!(
        conversation_id = %claim.conversation_id,
        active_segment_start_seq = claim.active_segment_start_seq,
        active_segment_end_seq = claim.active_segment_end_seq,
        re_enqueued,
        "Skipping stale segmentation job"
      );
      Ok(None)
    }
  }
}

async fn process_claimed_segment_range(
  claim: &SegmentationJobClaim,
  eof_identified: bool,
  claimed_messages: &[plastmem_core::ConversationMessage],
  ctx: &SegmentationContext<'_>,
) -> Result<(), AppError> {
  if claimed_messages.is_empty() {
    return Err(AppError::new(anyhow::anyhow!(
      "Segmentation claim has no messages"
    )));
  }

  let rule_output = temporal_rule_segmenter(claimed_messages)
    .map_err(|reason| AppError::new(anyhow::anyhow!(reason)))?;
  let (reviewed_segments, reviewed_boundaries) =
    primitive_review_llm_segmenter(claimed_messages, &rule_output).await?;
  let final_segments = temporal_boundary_review_llm_segmenter(
    claimed_messages,
    &reviewed_segments,
    &reviewed_boundaries,
  )
  .await?;

  let commit_plan = build_commit_plan(
    &final_segments,
    eof_identified,
    claim.active_segment_end_seq,
  )
  .map_err(|reason| AppError::new(anyhow::anyhow!(reason)))?;

  let created_at = Utc::now();
  let finalized_spans: Vec<EpisodeSpan> = commit_plan
    .finalized_segments
    .iter()
    .map(|segment| EpisodeSpan {
      conversation_id: claim.conversation_id,
      start_seq: segment.start_seq,
      end_seq: segment.end_seq,
      classification: map_classification(segment.classification.clone()),
      created_at,
    })
    .collect();

  commit_segmentation_job(
    claim,
    &finalized_spans,
    commit_plan.next_segment_start_seq,
    ctx.db,
  )
  .await?;

  enqueue_episode_creation_jobs(&finalized_spans, ctx.episode_creation_storage).await?;

  if !finalized_spans.is_empty() {
    enqueue_pending_reviews(
      claim.conversation_id,
      &extract_review_context(claimed_messages),
      ctx.db,
      ctx.review_storage,
    )
    .await?;
  }

  if commit_plan.next_segment_start_seq > claim.active_segment_start_seq {
    try_claim_and_enqueue_segmentation_job(
      claim.conversation_id,
      "commit_follow_up",
      ctx.db,
      ctx.segmentation_storage,
    )
    .await?;
  }

  Ok(())
}

fn build_commit_plan(
  segments: &[ReviewedSegment],
  eof_identified: bool,
  claimed_end_seq: i64,
) -> Result<CommitPlan, String> {
  if segments.is_empty() {
    return Err("Cannot build commit plan from empty segment list".to_owned());
  }

  if eof_identified {
    return Ok(CommitPlan {
      finalized_segments: segments.to_vec(),
      next_segment_start_seq: claimed_end_seq + 1,
    });
  }

  if segments.len() == 1 {
    return Ok(CommitPlan {
      finalized_segments: Vec::new(),
      next_segment_start_seq: segments[0].start_seq,
    });
  }

  let carry_over = segments
    .last()
    .ok_or_else(|| "Missing carry-over segment".to_owned())?;
  Ok(CommitPlan {
    finalized_segments: segments[..segments.len() - 1].to_vec(),
    next_segment_start_seq: carry_over.start_seq,
  })
}

// ──────────────────────────────────────────────────
// Utilities
// ──────────────────────────────────────────────────

fn extract_review_context(messages: &[plastmem_core::ConversationMessage]) -> Vec<Message> {
  messages
    .iter()
    .map(plastmem_core::ConversationMessage::to_message)
    .collect()
}

fn map_classification(classification: SegmentClassification) -> EpisodeClassification {
  match classification {
    SegmentClassification::LowInfo => EpisodeClassification::LowInfo,
    SegmentClassification::Informative => EpisodeClassification::Informative,
  }
}

// ──────────────────────────────────────────────────
// Side effects
// ──────────────────────────────────────────────────

async fn enqueue_pending_reviews(
  conversation_id: Uuid,
  context_messages: &[Message],
  db: &sea_orm::DatabaseConnection,
  review_storage: &PostgresStorage<MemoryReviewJob>,
) -> Result<(), AppError> {
  if !APP_ENV.enable_fsrs_review {
    return Ok(());
  }

  if let Some(pending_reviews) = take_pending_review_items(conversation_id, db).await? {
    let review_job = MemoryReviewJob {
      pending_reviews,
      context_messages: context_messages.to_vec(),
      reviewed_at: Utc::now(),
    };
    let mut storage = review_storage.clone();
    storage.push(review_job).await?;
  }

  Ok(())
}

// When the current worker attempt is about to end, ask core to re-check
// whether the conversation now exposes a fresh claimable segment range.
// If there is more unsegmented work after commit, stale-job recovery, or
// processing-error abort, enqueue one follow-up segmentation job to continue.
async fn try_claim_and_enqueue_segmentation_job(
  conversation_id: Uuid,
  trigger_context: &'static str,
  db: &sea_orm::DatabaseConnection,
  segmentation_storage: &PostgresStorage<EventSegmentationJob>,
) -> Result<bool, AppError> {
  if let Some(claim) = try_claim_segmentation_job(conversation_id, db).await? {
    let active_segment_start_seq = claim.active_segment_start_seq;
    let active_segment_end_seq = claim.active_segment_end_seq;
    let mut storage = segmentation_storage.clone();
    storage
      .push(EventSegmentationJob::from_claim(claim))
      .await?;
    tracing::info!(
      conversation_id = %conversation_id,
      active_segment_start_seq,
      active_segment_end_seq,
      trigger_context,
      "Enqueued segmentation job"
    );
    return Ok(true);
  }

  Ok(false)
}

async fn enqueue_episode_creation_jobs(
  finalized_spans: &[EpisodeSpan],
  episode_creation_storage: &PostgresStorage<EpisodeCreationJob>,
) -> Result<(), AppError> {
  if finalized_spans.is_empty() {
    return Ok(());
  }

  let jobs = finalized_spans
    .iter()
    .map(EpisodeCreationJob::from_span)
    .collect::<Vec<_>>();

  let mut storage = episode_creation_storage.clone();
  storage.push_bulk(jobs).await?;

  Ok(())
}
