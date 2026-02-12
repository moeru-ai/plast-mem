use std::ops::Deref;

use apalis::prelude::Data;
use chrono::{DateTime, Utc};
use fsrs::{DEFAULT_PARAMETERS, FSRS, MemoryState};
use plastmem_entities::episodic_memory;
use plastmem_shared::{AppError, fsrs::DESIRED_RETENTION};
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
) -> Result<(), AppError> {
  let db = db.deref();
  let fsrs = FSRS::new(Some(&DEFAULT_PARAMETERS))?;

  for memory_id in &job.memory_ids {
    let Some(model) = episodic_memory::Entity::find_by_id(*memory_id)
      .one(db)
      .await?
    else {
      continue; // memory was deleted, skip
    };

    let last_reviewed_at = model.last_reviewed_at.with_timezone(&Utc);
    if job.reviewed_at <= last_reviewed_at {
      continue; // skip stale job to avoid overwriting newer review
    }

    let days_elapsed = (job.reviewed_at - last_reviewed_at).num_days() as u32;

    let current_state = MemoryState {
      stability: model.stability,
      difficulty: model.difficulty,
    };

    let next_states = fsrs.next_states(Some(current_state), DESIRED_RETENTION, days_elapsed)?;

    // Auto GOOD review: being retrieved = reinforcement
    let new_state = next_states.good.memory;

    let mut active_model: episodic_memory::ActiveModel = model.into();
    active_model.stability = Set(new_state.stability);
    active_model.difficulty = Set(new_state.difficulty);
    active_model.last_reviewed_at = Set(job.reviewed_at.into());
    active_model.update(db).await?;
  }

  Ok(())
}
