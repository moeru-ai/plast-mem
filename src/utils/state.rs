use apalis_postgres::PostgresStorage;
use sea_orm::DatabaseConnection;

use plast_mem_worker::EventSegmentationJob;

#[derive(Clone)]
pub struct AppState {
  pub db: DatabaseConnection,
  pub job_storage: PostgresStorage<EventSegmentationJob>,
}

impl AppState {
  pub fn new(db: DatabaseConnection, job_storage: PostgresStorage<EventSegmentationJob>) -> Self {
    Self { db, job_storage }
  }
}
