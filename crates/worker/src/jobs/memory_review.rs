use std::ops::Deref;

use apalis::prelude::Data;
use chrono::{DateTime, Utc};
use fsrs::{FSRS, MemoryState};
use plast_mem_db_schema::episodic_memory;
use plast_mem_shared::AppError;
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::jobs::WorkerError;

/// Target retention probability (90%).
const DESIRED_RETENTION: f32 = 0.9;

/// Job to update FSRS parameters for retrieved memories.
///
/// Currently performs an automatic GOOD review for each retrieved memory,
/// reinforcing memories that are actively being recalled.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MemoryReviewJob {
  pub memory_ids: Vec<Uuid>,
  pub reviewed_at: DateTime<Utc>,
}

pub async fn process_memory_review(
  job: MemoryReviewJob,
  db: Data<DatabaseConnection>,
) -> Result<(), WorkerError> {
  let db = db.deref();
  let fsrs =
    FSRS::new(None).map_err(|e| WorkerError::from(AppError::new(anyhow::anyhow!("{e}"))))?;

  for memory_id in &job.memory_ids {
    let Some(model) = episodic_memory::Entity::find_by_id(*memory_id)
      .one(db)
      .await
      .map_err(AppError::from)?
    else {
      continue; // memory was deleted, skip
    };

    let last_reviewed_at = model.last_reviewed_at.with_timezone(&Utc);
    if job.reviewed_at <= last_reviewed_at {
      continue; // skip stale job to avoid overwriting newer review
    }

    let days_elapsed = (job.reviewed_at - last_reviewed_at).num_seconds().max(0) as u32 / 86400;

    let current_state = MemoryState {
      stability: model.stability,
      difficulty: model.difficulty,
    };

    let next_states = fsrs
      .next_states(Some(current_state), DESIRED_RETENTION, days_elapsed)
      .map_err(|e| WorkerError::from(AppError::new(anyhow::anyhow!("{e}"))))?;

    // Auto GOOD review: being retrieved = reinforcement
    let new_state = next_states.good.memory;

    let mut active_model: episodic_memory::ActiveModel = model.into();
    active_model.stability = Set(new_state.stability);
    active_model.difficulty = Set(new_state.difficulty);
    active_model.last_reviewed_at = Set(job.reviewed_at.into());
    active_model.update(db).await.map_err(AppError::from)?;
  }

  Ok(())
}
