use apalis::prelude::TaskSink;
use axum::{Json, extract::{Query, State}};
use chrono::Utc;
use plastmem_core::MessageQueue;
use plastmem_shared::{AppError, Message};
use plastmem_worker::EventSegmentationJob;
use sea_orm::{DatabaseConnection, DbBackend, FromQueryResult, Statement};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::utils::AppState;

use super::add_message::AddMessageMessage;

// ──────────────────────────────────────────────────
// Flush
// ──────────────────────────────────────────────────

#[derive(Debug, Deserialize, ToSchema)]
pub struct BenchmarkFlush {
  pub conversation_id: Uuid,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BenchmarkFlushResult {
  /// Whether a flush job was enqueued (false if queue was already empty).
  pub enqueued: bool,
}

fn default_force_process() -> bool {
  true
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct BenchmarkAddMessages {
  pub conversation_id: Uuid,
  pub messages: Vec<AddMessageMessage>,
  /// Enqueue forced segmentation immediately after append.
  #[serde(default = "default_force_process")]
  pub force_process: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BenchmarkAddMessagesResult {
  pub accepted: usize,
  pub enqueued: bool,
}

/// Append a batch of messages and optionally enqueue segmentation immediately.
/// Intended for benchmark ingestion to avoid per-message API round-trips.
#[utoipa::path(
  post,
  path = "/api/v0/benchmark/add_messages",
  request_body = BenchmarkAddMessages,
  responses(
    (status = 200, description = "Batch add result", body = BenchmarkAddMessagesResult),
    (status = 400, description = "Invalid request")
  )
)]
#[axum::debug_handler]
#[tracing::instrument(skip(state), fields(conversation_id = %payload.conversation_id, messages = payload.messages.len()))]
pub async fn benchmark_add_messages(
  State(state): State<AppState>,
  Json(payload): Json<BenchmarkAddMessages>,
) -> Result<Json<BenchmarkAddMessagesResult>, AppError> {
  let BenchmarkAddMessages {
    conversation_id: id,
    messages,
    force_process,
  } = payload;

  let mut normalized = Vec::with_capacity(messages.len());
  for msg in messages {
    if msg.content.is_empty() {
      return Err(AppError::new(anyhow::anyhow!(
        "Message content cannot be empty"
      )));
    }

    normalized.push(Message {
      role: msg.role,
      content: msg.content,
      timestamp: msg.timestamp.unwrap_or_else(Utc::now),
    });
  }

  let accepted = normalized.len();
  let msg_count = MessageQueue::push_batch(id, normalized, &state.db).await?;
  let mut enqueued = false;

  if msg_count > 0 {
    if force_process {
      // Force-claim the current queue for immediate segmentation.
      MessageQueue::clear_fence(id, &state.db).await?;
      if MessageQueue::try_set_fence(id, msg_count, &state.db).await? {
        let mut job_storage = state.job_storage.clone();
        job_storage
          .push(EventSegmentationJob {
            conversation_id: id,
            fence_count: msg_count,
            force_process: true,
            keep_tail_segment: false,
          })
          .await?;
      }
      enqueued = true;
    } else if let Some(check) = MessageQueue::check(id, msg_count, &state.db).await? {
      let mut job_storage = state.job_storage.clone();
      job_storage
        .push(EventSegmentationJob {
          conversation_id: id,
          fence_count: check.fence_count,
          force_process: check.force_process,
          keep_tail_segment: false,
        })
        .await?;
      enqueued = true;
    }
  }

  Ok(Json(BenchmarkAddMessagesResult { accepted, enqueued }))
}

/// Force-flush the message queue for a conversation.
///
/// Clears any in-progress fence and enqueues an `EventSegmentationJob` with
/// `force_process = true`, ensuring all remaining messages are processed into
/// episodic memories regardless of normal trigger thresholds.
/// Intended for use by the benchmark runner after ingestion.
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

  let msg_count = get_message_count(id, &state.db).await?;

  if msg_count == 0 {
    return Ok(Json(BenchmarkFlushResult { enqueued: false }));
  }

  // Force-clear any existing fence so we can set a fresh one.
  MessageQueue::clear_fence(id, &state.db).await?;

  // Set the fence to current count to claim ownership.
  if !MessageQueue::try_set_fence(id, msg_count, &state.db).await? {
    // Another concurrent flush got there first; still return enqueued = true.
    return Ok(Json(BenchmarkFlushResult { enqueued: true }));
  }

  let mut job_storage = state.job_storage.clone();
  job_storage
    .push(EventSegmentationJob {
      conversation_id: id,
      fence_count: msg_count,
      force_process: true,
      keep_tail_segment: false,
    })
    .await?;

  tracing::info!(conversation_id = %id, msg_count, "Benchmark flush enqueued");

  Ok(Json(BenchmarkFlushResult { enqueued: true }))
}

// ──────────────────────────────────────────────────
// Job status
// ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct JobStatusQuery {
  pub conversation_id: Uuid,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BenchmarkJobStatus {
  /// Number of messages still pending in the queue (not yet segmented).
  pub messages_pending: i32,
  /// Whether a segmentation job fence is currently active.
  pub fence_active: bool,
  /// Number of active (Pending or Running) Apalis jobs for this conversation.
  /// Covers EventSegmentationJob and PredictCalibrateJob.
  pub apalis_active: i64,
  /// True when the message queue is empty, no fence is active, and no Apalis jobs are active.
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
struct QueueStatusRow {
  messages_pending: i32,
  fence_active: bool,
}

#[derive(Debug, FromQueryResult)]
struct ApalisActiveRow {
  active: i64,
}

async fn get_queue_status(
  id: Uuid,
  db: &DatabaseConnection,
) -> Result<BenchmarkJobStatus, AppError> {
  let queue_sql = "SELECT \
    COALESCE(jsonb_array_length(messages), 0)::int AS messages_pending, \
    (in_progress_fence IS NOT NULL) AS fence_active \
    FROM message_queue WHERE id = $1";

  let queue_row = QueueStatusRow::find_by_statement(Statement::from_sql_and_values(
    DbBackend::Postgres,
    queue_sql,
    [id.into()],
  ))
  .one(db)
  .await?;

  // Count active Apalis jobs whose payload contains this conversation_id.
  // Covers EventSegmentationJob and SemanticConsolidationJob (both embed conversation_id).
  let apalis_sql = "SELECT COUNT(*)::bigint AS active FROM apalis.jobs \
    WHERE status IN ('Pending', 'Running') \
    AND convert_from(job, 'UTF8')::jsonb->>'conversation_id' = $1";

  let apalis_row = ApalisActiveRow::find_by_statement(Statement::from_sql_and_values(
    DbBackend::Postgres,
    apalis_sql,
    [id.to_string().into()],
  ))
  .one(db)
  .await?;

  let apalis_active = apalis_row.map_or(0, |r| r.active);

  let status = queue_row.map_or(
    BenchmarkJobStatus {
      messages_pending: 0,
      fence_active: false,
      apalis_active,
      done: apalis_active == 0,
    },
    |r| BenchmarkJobStatus {
      messages_pending: r.messages_pending,
      fence_active: r.fence_active,
      apalis_active,
      done: !r.fence_active && r.messages_pending == 0 && apalis_active == 0,
    },
  );

  Ok(status)
}

#[derive(Debug, FromQueryResult)]
struct MessageCountRow {
  count: i32,
}

async fn get_message_count(id: Uuid, db: &DatabaseConnection) -> Result<i32, AppError> {
  let sql = "SELECT COALESCE(jsonb_array_length(messages), 0)::int AS count \
             FROM message_queue WHERE id = $1";

  let row = MessageCountRow::find_by_statement(Statement::from_sql_and_values(
    DbBackend::Postgres,
    sql,
    [id.into()],
  ))
  .one(db)
  .await?;

  Ok(row.map_or(0, |r| r.count))
}
