use apalis_postgres::PostgresStorage;
use sea_orm::DatabaseConnection;

use plastmem_worker::{EventSegmentationJob, MemoryReviewJob, PredictCalibrateJob};

#[derive(Clone)]
pub struct AppState {
  pub db: DatabaseConnection,
  pub segmentation_job_storage: PostgresStorage<EventSegmentationJob>,
  pub review_job_storage: PostgresStorage<MemoryReviewJob>,
  pub predict_calibrate_job_storage: PostgresStorage<PredictCalibrateJob>,
}

impl AppState {
  #[must_use]
  pub fn new(
    db: DatabaseConnection,
    segmentation_job_storage: PostgresStorage<EventSegmentationJob>,
    review_job_storage: PostgresStorage<MemoryReviewJob>,
    predict_calibrate_job_storage: PostgresStorage<PredictCalibrateJob>,
  ) -> Self {
    Self {
      db,
      segmentation_job_storage,
      review_job_storage,
      predict_calibrate_job_storage,
    }
  }
}
