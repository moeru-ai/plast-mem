use apalis_postgres::PostgresStorage;
use sea_orm::DatabaseConnection;

use plast_mem_worker::WorkerJob;

#[derive(Clone)]
pub struct AppState {
  pub db: DatabaseConnection,
  pub job_storage: PostgresStorage<WorkerJob>,
}

impl AppState {
  pub fn new(db: DatabaseConnection, job_storage: PostgresStorage<WorkerJob>) -> Self {
    Self { db, job_storage }
  }
}
