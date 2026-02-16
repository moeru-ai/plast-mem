use apalis_postgres::PostgresStorage;
use sea_orm::DatabaseConnection;

use plastmem_worker::EventSegmentationJob;

#[derive(Clone)]
pub struct AppState {
  pub db: DatabaseConnection,
  pub job_storage: PostgresStorage<EventSegmentationJob>,
}

impl AppState {
  #[must_use]
  pub const fn new(
    db: DatabaseConnection,
    job_storage: PostgresStorage<EventSegmentationJob>,
  ) -> Self {
    Self { db, job_storage }
  }
}
