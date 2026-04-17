use std::time::Duration;

use apalis::{
  layers::WorkerBuilderExt,
  prelude::{Monitor, WorkerBuilder},
};
use apalis_postgres::PostgresStorage;
use plastmem_shared::APP_ENV;
use plastmem_shared::AppError;
use sea_orm::DatabaseConnection;

pub mod jobs;
pub use jobs::EpisodeCreationJob;
pub use jobs::EventSegmentationJob;
pub use jobs::MemoryReviewJob;
pub use jobs::PredictCalibrateJob;
use jobs::{
  WorkerError, process_episode_creation, process_event_segmentation, process_memory_review,
  process_predict_calibrate,
};

pub async fn worker(
  db: &DatabaseConnection,
  segmentation_backend: PostgresStorage<EventSegmentationJob>,
  episode_creation_backend: PostgresStorage<EpisodeCreationJob>,
  review_backend: PostgresStorage<MemoryReviewJob>,
  semantic_backend: PostgresStorage<PredictCalibrateJob>,
) -> Result<(), AppError> {
  let db = db.clone();

  Monitor::new()
    .register({
      let db = db.clone();
      let segmentation_backend = segmentation_backend.clone();
      let episode_creation_backend = episode_creation_backend.clone();
      let review_backend = review_backend.clone();
      move |_run_id| {
        WorkerBuilder::new("event-segmentation")
          .backend(segmentation_backend.clone())
          .concurrency(1)
          .enable_tracing()
          .data(db.clone())
          .data(segmentation_backend.clone())
          .data(episode_creation_backend.clone())
          .data(review_backend.clone())
          .build(
            move |job, data, segmentation_storage, episode_creation_storage, review_storage| async move {
              process_event_segmentation(
                job,
                data,
                segmentation_storage,
                episode_creation_storage,
                review_storage,
              )
              .await
              .map_err(WorkerError::from)
            },
          )
      }
    })
    .register({
      let db = db.clone();
      let semantic_backend = semantic_backend.clone();
      move |_run_id| {
        WorkerBuilder::new("episode-creation")
          .backend(episode_creation_backend.clone())
          .concurrency(APP_ENV.predict_calibrate_concurrency)
          .enable_tracing()
          .data(db.clone())
          .data(semantic_backend.clone())
          .build(move |job, data, predict_storage| async move {
            process_episode_creation(job, data, predict_storage)
              .await
              .map_err(WorkerError::from)
          })
      }
    })
    .register({
      let db = db.clone();
      move |_run_id| {
        WorkerBuilder::new("memory-review")
          .backend(review_backend.clone())
          .enable_tracing()
          .data(db.clone())
          .build(move |job, data| async move {
            process_memory_review(job, data)
              .await
              .map_err(WorkerError::from)
          })
      }
    })
    .register({
      let db = db.clone();
      move |_run_id| {
        WorkerBuilder::new("predict-calibrate")
          .backend(semantic_backend.clone())
          .concurrency(APP_ENV.predict_calibrate_concurrency)
          .enable_tracing()
          .data(db.clone())
          .build(move |job, data| async move {
            process_predict_calibrate(job, data)
              .await
              .map_err(WorkerError::from)
          })
      }
    })
    .shutdown_timeout(Duration::from_secs(5))
    .run_with_signal(tokio::signal::ctrl_c())
    .await?;

  Ok(())
}
