use std::time::Duration;

use apalis::{
  layers::WorkerBuilderExt,
  prelude::{Monitor, WorkerBuilder},
};
use apalis_postgres::PostgresStorage;
use plast_mem_shared::AppError;
use sea_orm::DatabaseConnection;

pub mod jobs;
pub use jobs::EventSegmentationJob;
use jobs::process_event_segmentation;

pub async fn worker(
  db: &DatabaseConnection,
  backend: PostgresStorage<EventSegmentationJob>,
) -> Result<(), AppError> {
  let db = db.clone();

  Monitor::new()
    .register(move |_run_id| {
      WorkerBuilder::new("event-segmentation")
        .backend(backend.clone())
        .enable_tracing()
        .data(db.clone())
        .build(process_event_segmentation)
    })
    .shutdown_timeout(Duration::from_secs(5))
    .run_with_signal(tokio::signal::ctrl_c())
    .await
    .map_err(|err| AppError::new(anyhow::Error::new(err)))?;

  Ok(())
}
