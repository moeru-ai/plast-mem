use std::time::Duration;

use anyhow::anyhow;
use apalis::prelude::{BoxDynError, Monitor, WorkerBuilder, WorkerContext};
use apalis_postgres::PostgresStorage;
use sea_orm::DatabaseConnection;

use crate::utils::AppError;

pub async fn worker(db: &DatabaseConnection) -> Result<(), AppError> {
  let backend = PostgresStorage::new(db.get_postgres_connection_pool());

  async fn send_reminder(item: usize, _wrk: WorkerContext) -> Result<(), BoxDynError> {
    if item.is_multiple_of(3) {
      println!("Reminding about item: {} but failing", item);
      return Err(anyhow!("Failed to send reminder").into());
    }
    println!("Reminding about item: {}", item);
    Ok(())
  }

  Monitor::new()
    .register(move |_run_id| {
      WorkerBuilder::new("plast-mem-worker")
        .backend(backend.clone())
        .build(send_reminder)
    })
    .shutdown_timeout(Duration::from_secs(5))
    .run_with_signal(tokio::signal::ctrl_c())
    .await?;

  Ok(())
}
