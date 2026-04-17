use apalis_postgres::PostgresStorage;
use sea_orm::DatabaseConnection;

use plastmem_worker::{
  EpisodeCreationJob, EventSegmentationJob, MemoryReviewJob, PredictCalibrateJob,
};

#[derive(Clone)]
pub struct AppState {
  pub db: DatabaseConnection,
  pub segmentation_job_storage: PostgresStorage<EventSegmentationJob>,
  pub episode_creation_job_storage: PostgresStorage<EpisodeCreationJob>,
  pub review_job_storage: PostgresStorage<MemoryReviewJob>,
  pub predict_calibrate_job_storage: PostgresStorage<PredictCalibrateJob>,
}

impl AppState {
  #[must_use]
  #[allow(clippy::missing_const_for_fn)]
  pub fn new(
    db: DatabaseConnection,
    segmentation_job_storage: PostgresStorage<EventSegmentationJob>,
    episode_creation_job_storage: PostgresStorage<EpisodeCreationJob>,
    review_job_storage: PostgresStorage<MemoryReviewJob>,
    predict_calibrate_job_storage: PostgresStorage<PredictCalibrateJob>,
  ) -> Self {
    Self {
      db,
      segmentation_job_storage,
      episode_creation_job_storage,
      review_job_storage,
      predict_calibrate_job_storage,
    }
  }
}
