use std::fmt::Write;

use axum::{Json, extract::State};
use chrono::Utc;
use chrono_humanize::HumanTime;
use plastmem_core::EpisodicMemory;
use plastmem_entities::episodic_memory;
use plastmem_shared::AppError;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::utils::AppState;

#[derive(Deserialize, ToSchema)]
pub struct RecentMemory {
  /// Conversation ID to filter memories by
  pub conversation_id: Uuid,
  /// Limit to memories from the last N days (optional)
  /// If not provided, returns the most recent memories up to `limit`
  pub days_limit: Option<u64>,
  /// Maximum memories to return (default: 10, max: 100)
  #[serde(default = "default_limit")]
  pub limit: u64,
}

const fn default_limit() -> u64 {
  10
}

fn sanitize_limit(value: u64) -> u64 {
  if value > 0 && value <= 100 { value } else { 10 }
}

/// Retrieve recent memories in raw JSON format (newest first)
#[utoipa::path(
  post,
  path = "/api/v0/recent_memory/raw",
  request_body = RecentMemory,
  responses(
    (status = 200, description = "Recent episodic memories", body = Vec<EpisodicMemory>),
  )
)]
#[axum::debug_handler]
pub async fn recent_memory_raw(
  State(state): State<AppState>,
  Json(payload): Json<RecentMemory>,
) -> Result<Json<Vec<EpisodicMemory>>, AppError> {
  let limit = sanitize_limit(payload.limit);

  // Build query using SeaORM directly
  let mut query = episodic_memory::Entity::find()
    .filter(episodic_memory::Column::ConversationId.eq(payload.conversation_id));

  // Apply days filter if provided
  if let Some(days) = payload.days_limit {
    let since = Utc::now() - chrono::Duration::days(days as i64);
    query = query.filter(episodic_memory::Column::CreatedAt.gte(since));
  }

  // Order by created_at DESC (newest first) and limit
  let models = query
    .order_by_desc(episodic_memory::Column::CreatedAt)
    .limit(limit)
    .all(&state.db)
    .await?;

  let memories: Result<Vec<_>, _> = models.into_iter().map(EpisodicMemory::from_model).collect();

  Ok(Json(memories?))
}

/// Retrieve recent memories formatted as markdown for LLM consumption.
/// Returns only summaries, no full message details.
#[utoipa::path(
  post,
  path = "/api/v0/recent_memory",
  request_body = RecentMemory,
  responses(
    (status = 200, description = "Markdown formatted recent memories", body = String),
  )
)]
#[axum::debug_handler]
pub async fn recent_memory(
  State(state): State<AppState>,
  Json(payload): Json<RecentMemory>,
) -> Result<String, AppError> {
  let limit = sanitize_limit(payload.limit);

  // Build query using SeaORM directly
  let mut query = episodic_memory::Entity::find()
    .filter(episodic_memory::Column::ConversationId.eq(payload.conversation_id));

  // Apply days filter if provided
  if let Some(days) = payload.days_limit {
    let since = Utc::now() - chrono::Duration::days(days as i64);
    query = query.filter(episodic_memory::Column::CreatedAt.gte(since));
  }

  // Order by created_at DESC (newest first) and limit
  let models = query
    .order_by_desc(episodic_memory::Column::CreatedAt)
    .limit(limit)
    .all(&state.db)
    .await?;

  let now = Utc::now();
  let mut out = String::new();

  if models.is_empty() {
    let _ = writeln!(out, "No recent memories found.");
    return Ok(out);
  }

  let _ = writeln!(out, "## Recent Memories\n");

  for model in models {
    let mem = EpisodicMemory::from_model(model)?;
    let key_moment = if mem.surprise >= 0.7 {
      " (key moment)"
    } else {
      ""
    };
    let header = if mem.title.is_empty() {
      "Memory".to_string()
    } else {
      mem.title.clone()
    };

    let duration = now.signed_duration_since(mem.created_at);
    let time_str = HumanTime::from(duration);

    let _ = writeln!(out, "### {header}{key_moment}");
    let _ = writeln!(out, "**When:** {time_str}");
    let _ = writeln!(out, "**Summary:** {}\n", mem.summary);
  }

  Ok(out.trim_end().to_string())
}
