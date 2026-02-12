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
pub use jobs::MemoryReviewJob;
use jobs::{process_event_segmentation, process_memory_review};

pub async fn worker(
  db: &DatabaseConnection,
  segmentation_backend: PostgresStorage<EventSegmentationJob>,
  review_backend: PostgresStorage<MemoryReviewJob>,
) -> Result<(), AppError> {
  let db = db.clone();

  Monitor::new()
    .register({
      let db = db.clone();
      move |_run_id| {
        WorkerBuilder::new("event-segmentation")
          .backend(segmentation_backend.clone())
          .enable_tracing()
          .data(db.clone())
          .build(process_event_segmentation)
      }
    })
    .register({
      let db = db.clone();
      move |_run_id| {
        WorkerBuilder::new("memory-review")
          .backend(review_backend.clone())
          .enable_tracing()
          .data(db.clone())
          .build(process_memory_review)
      }
    })
    .shutdown_timeout(Duration::from_secs(5))
    .run_with_signal(tokio::signal::ctrl_c())
    .await
    .map_err(|err| AppError::new(anyhow::Error::new(err)))?;

  Ok(())
}
