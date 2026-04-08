use apalis::prelude::TaskSink;
use axum::{
  Json,
  extract::{Query, State},
};
use plastmem_core::{ADD_BACKPRESSURE_LIMIT, FENCE_TTL_MINUTES, MessageQueue};
use plastmem_shared::AppError;
use plastmem_worker::EventSegmentationJob;
use sea_orm::{DatabaseConnection, DbBackend, FromQueryResult, Statement};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::utils::AppState;

// ──────────────────────────────────────────────────
// Flush
// ──────────────────────────────────────────────────

#[derive(Debug, Deserialize, ToSchema)]
pub struct BenchmarkFlush {
  pub conversation_id: Uuid,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BenchmarkFlushResult {
  pub enqueued: bool,
  pub reason: String,
}

#[utoipa::path(
  post,
  path = "/api/v0/benchmark/flush",
  request_body = BenchmarkFlush,
  responses(
    (status = 200, description = "Flush result", body = BenchmarkFlushResult),
    (status = 400, description = "Invalid request")
  )
)]
#[axum::debug_handler]
#[tracing::instrument(skip(state), fields(conversation_id = %payload.conversation_id))]
pub async fn benchmark_flush(
  State(state): State<AppState>,
  Json(payload): Json<BenchmarkFlush>,
) -> Result<Json<BenchmarkFlushResult>, AppError> {
  let id = payload.conversation_id;
  let status = get_queue_status(id, &state.db).await?;

  if status.messages_pending == 0 {
    return Ok(Json(BenchmarkFlushResult {
      enqueued: false,
      reason: "empty".to_owned(),
    }));
  }

  if !status.flushable {
    return Ok(Json(BenchmarkFlushResult {
      enqueued: false,
      reason: "busy".to_owned(),
    }));
  }

  if !MessageQueue::try_set_fence(id, status.messages_pending, &state.db).await? {
    return Ok(Json(BenchmarkFlushResult {
      enqueued: false,
      reason: "busy".to_owned(),
    }));
  }

  let mut job_storage = state.segmentation_job_storage.clone();
  job_storage
    .push(EventSegmentationJob {
      conversation_id: id,
      fence_count: status.messages_pending,
      force_process: true,
      keep_tail_segment: false,
    })
    .await?;

  tracing::info!(
    conversation_id = %id,
    msg_count = status.messages_pending,
    "Benchmark flush enqueued"
  );

  Ok(Json(BenchmarkFlushResult {
    enqueued: true,
    reason: "enqueued".to_owned(),
  }))
}

// ──────────────────────────────────────────────────
// Job status
// ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct JobStatusQuery {
  pub conversation_id: Uuid,
}

#[derive(Debug, Serialize, ToSchema)]
#[allow(clippy::struct_excessive_bools)]
pub struct BenchmarkJobStatus {
  pub messages_pending: i32,
  pub fence_active: bool,
  pub segmentation_jobs_active: i64,
  pub predict_calibrate_jobs_active: i64,
  pub admissible_for_add: bool,
  pub flushable: bool,
  pub done: bool,
}

/// Query the processing status of a conversation's message queue.
///
/// Used by the benchmark runner to poll until all ingested messages have been
/// processed into episodic memories before running evaluation.
#[utoipa::path(
  get,
  path = "/api/v0/benchmark/job_status",
  params(
    ("conversation_id" = Uuid, Query, description = "Conversation ID to check")
  ),
  responses(
    (status = 200, description = "Job status", body = BenchmarkJobStatus),
    (status = 400, description = "Invalid request")
  )
)]
#[axum::debug_handler]
#[tracing::instrument(skip(state), fields(conversation_id = %query.conversation_id))]
pub async fn benchmark_job_status(
  State(state): State<AppState>,
  Query(query): Query<JobStatusQuery>,
) -> Result<Json<BenchmarkJobStatus>, AppError> {
  let id = query.conversation_id;
  let status = get_queue_status(id, &state.db).await?;
  Ok(Json(status))
}

// ──────────────────────────────────────────────────
// Internal helpers
// ──────────────────────────────────────────────────

#[derive(Debug, FromQueryResult)]
struct JobCountsRow {
  segmentation_jobs_active: i64,
  predict_calibrate_jobs_active: i64,
}

async fn get_queue_status(
  id: Uuid,
  db: &DatabaseConnection,
) -> Result<BenchmarkJobStatus, AppError> {
  let mut queue_status = MessageQueue::get_processing_status(id, db).await?;
  if queue_status.fence_active && MessageQueue::clear_stale_fence(id, FENCE_TTL_MINUTES, db).await?
  {
    queue_status = MessageQueue::get_processing_status(id, db).await?;
  }

  let jobs_sql = "SELECT \
    COUNT(*) FILTER (WHERE status IN ('Pending', 'Running') AND job_type LIKE '%EventSegmentationJob%' AND convert_from(job, 'UTF8')::jsonb->>'conversation_id' = $1)::bigint AS segmentation_jobs_active, \
    COUNT(*) FILTER (WHERE status IN ('Pending', 'Running') AND job_type LIKE '%PredictCalibrateJob%' AND convert_from(job, 'UTF8')::jsonb->>'conversation_id' = $1)::bigint AS predict_calibrate_jobs_active \
    FROM apalis.jobs";

  let jobs_row = JobCountsRow::find_by_statement(Statement::from_sql_and_values(
    DbBackend::Postgres,
    jobs_sql,
    [id.to_string().into()],
  ))
  .one(db)
  .await?;

  let jobs = jobs_row.unwrap_or(JobCountsRow {
    segmentation_jobs_active: 0,
    predict_calibrate_jobs_active: 0,
  });

  let admissible_for_add =
    !queue_status.fence_active || queue_status.messages_pending < ADD_BACKPRESSURE_LIMIT;
  let flushable = queue_status.messages_pending > 0
    && !queue_status.fence_active
    && jobs.segmentation_jobs_active == 0;
  let done = queue_status.messages_pending == 0
    && !queue_status.fence_active
    && jobs.segmentation_jobs_active == 0
    && jobs.predict_calibrate_jobs_active == 0;

  Ok(BenchmarkJobStatus {
    messages_pending: queue_status.messages_pending,
    fence_active: queue_status.fence_active,
    segmentation_jobs_active: jobs.segmentation_jobs_active,
    predict_calibrate_jobs_active: jobs.predict_calibrate_jobs_active,
    admissible_for_add,
    flushable,
    done,
  })
}
