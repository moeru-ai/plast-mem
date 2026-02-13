use apalis_postgres::PostgresStorage;
use sea_orm::DatabaseConnection;

use plastmem_worker::{EventSegmentationJob, MemoryReviewJob};

#[derive(Clone)]
pub struct AppState {
  pub db: DatabaseConnection,
  pub job_storage: PostgresStorage<EventSegmentationJob>,
  pub review_job_storage: PostgresStorage<MemoryReviewJob>,
}

impl AppState {
  #[must_use]
  pub const fn new(
    db: DatabaseConnection,
    job_storage: PostgresStorage<EventSegmentationJob>,
    review_job_storage: PostgresStorage<MemoryReviewJob>,
  ) -> Self {
    Self {
      db,
      job_storage,
      review_job_storage,
    }
  }
}
