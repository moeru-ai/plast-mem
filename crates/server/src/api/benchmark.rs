use axum::{
  Json,
  extract::{Query, State},
};
use plastmem_core::{get_segmentation_processing_status, recover_stale_segmentation_job};
use plastmem_shared::AppError;
use sea_orm::{DatabaseConnection, DbBackend, FromQueryResult, Statement};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::utils::AppState;

// ──────────────────────────────────────────────────
// Job status
// ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct JobStatusQuery {
  pub conversation_id: Uuid,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BenchmarkJobStatus {
  pub messages_pending: i32,
  pub fence_active: bool,
  pub segmentation_jobs_active: i64,
  pub episode_creation_jobs_active: i64,
  pub predict_calibrate_jobs_active: i64,
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
  episode_creation_jobs_active: i64,
  predict_calibrate_jobs_active: i64,
}

async fn get_queue_status(
  id: Uuid,
  db: &DatabaseConnection,
) -> Result<BenchmarkJobStatus, AppError> {
  recover_stale_segmentation_job(id, db).await?;
  let segmentation_status = get_segmentation_processing_status(id, db).await?;

  let jobs_sql = "SELECT \
    COUNT(*) FILTER (WHERE status IN ('Pending', 'Running') AND job_type LIKE '%EventSegmentationJob%' AND convert_from(job, 'UTF8')::jsonb->>'conversation_id' = $1)::bigint AS segmentation_jobs_active, \
    COUNT(*) FILTER (WHERE status IN ('Pending', 'Running') AND job_type LIKE '%EpisodeCreationJob%' AND convert_from(job, 'UTF8')::jsonb->>'conversation_id' = $1)::bigint AS episode_creation_jobs_active, \
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
    episode_creation_jobs_active: 0,
    predict_calibrate_jobs_active: 0,
  });

  let done = segmentation_status.pending_message_count == 0
    && !segmentation_status.active
    && jobs.segmentation_jobs_active == 0
    && jobs.episode_creation_jobs_active == 0
    && jobs.predict_calibrate_jobs_active == 0;

  Ok(BenchmarkJobStatus {
    messages_pending: i32::try_from(segmentation_status.pending_message_count).unwrap_or(i32::MAX),
    fence_active: segmentation_status.active,
    segmentation_jobs_active: jobs.segmentation_jobs_active,
    episode_creation_jobs_active: jobs.episode_creation_jobs_active,
    predict_calibrate_jobs_active: jobs.predict_calibrate_jobs_active,
    done,
  })
}
