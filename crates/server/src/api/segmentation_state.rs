use axum::{
  Json,
  extract::{Query, State},
};
use plastmem_core::{
  SEGMENTATION_IN_PROGRESS_TTL_MINUTES, clear_stale_in_progress, get_processing_status,
};
use plastmem_shared::AppError;
use sea_orm::{DatabaseConnection, DbBackend, FromQueryResult, Statement};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::utils::AppState;

#[derive(Debug, Deserialize, ToSchema)]
pub struct SegmentationStateQuery {
  pub conversation_id: Uuid,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SegmentationStateStatus {
  pub messages_pending: i64,
  pub fence_active: bool,
  pub eof_seen: bool,
  pub next_unsegmented_seq: i64,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub last_seen_seq: Option<i64>,
  pub segmentation_jobs_active: i64,
  pub predict_calibrate_jobs_active: i64,
  pub admissible_for_add: bool,
  pub done: bool,
}

#[derive(Debug, FromQueryResult)]
struct JobCountsRow {
  segmentation_jobs_active: i64,
  predict_calibrate_jobs_active: i64,
}

#[utoipa::path(
  get,
  path = "/api/v1/segmentation_state",
  params(
    ("conversation_id" = Uuid, Query, description = "Conversation ID to inspect")
  ),
  responses(
    (status = 200, description = "Segmentation state", body = SegmentationStateStatus),
    (status = 400, description = "Invalid request")
  )
)]
#[axum::debug_handler]
#[tracing::instrument(skip(state), fields(conversation_id = %query.conversation_id))]
pub async fn segmentation_state(
  State(state): State<AppState>,
  Query(query): Query<SegmentationStateQuery>,
) -> Result<Json<SegmentationStateStatus>, AppError> {
  let status = get_segmentation_status(query.conversation_id, &state.db).await?;
  Ok(Json(status))
}

async fn get_segmentation_status(
  conversation_id: Uuid,
  db: &DatabaseConnection,
) -> Result<SegmentationStateStatus, AppError> {
  let mut processing_status = get_processing_status(conversation_id, db).await?;
  if processing_status.fence_active
    && clear_stale_in_progress(conversation_id, SEGMENTATION_IN_PROGRESS_TTL_MINUTES, db).await?
  {
    processing_status = get_processing_status(conversation_id, db).await?;
  }

  let jobs_sql = "SELECT \
    COUNT(*) FILTER (WHERE status IN ('Pending', 'Running') AND job_type LIKE '%EventSegmentationJob%' AND convert_from(job, 'UTF8')::jsonb->>'conversation_id' = $1)::bigint AS segmentation_jobs_active, \
    COUNT(*) FILTER (WHERE status IN ('Pending', 'Running') AND job_type LIKE '%PredictCalibrateJob%' AND convert_from(job, 'UTF8')::jsonb->>'conversation_id' = $1)::bigint AS predict_calibrate_jobs_active \
    FROM apalis.jobs";

  let jobs_row = JobCountsRow::find_by_statement(Statement::from_sql_and_values(
    DbBackend::Postgres,
    jobs_sql,
    [conversation_id.to_string().into()],
  ))
  .one(db)
  .await?;

  let jobs = jobs_row.unwrap_or(JobCountsRow {
    segmentation_jobs_active: 0,
    predict_calibrate_jobs_active: 0,
  });

  let admissible_for_add = !processing_status.fence_active
    || processing_status.messages_pending < plastmem_core::ADD_BACKPRESSURE_LIMIT;
  let done = processing_status.messages_pending == 0
    && !processing_status.fence_active
    && jobs.segmentation_jobs_active == 0
    && jobs.predict_calibrate_jobs_active == 0;

  Ok(SegmentationStateStatus {
    messages_pending: processing_status.messages_pending,
    fence_active: processing_status.fence_active,
    eof_seen: processing_status.eof_seen,
    next_unsegmented_seq: processing_status.next_unsegmented_seq,
    last_seen_seq: processing_status.last_seen_seq,
    segmentation_jobs_active: jobs.segmentation_jobs_active,
    predict_calibrate_jobs_active: jobs.predict_calibrate_jobs_active,
    admissible_for_add,
    done,
  })
}
