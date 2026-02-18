mod event_segmentation;
pub use event_segmentation::*;

mod memory_review;
pub use memory_review::*;

mod semantic_extraction;
pub use semantic_extraction::*;

use plastmem_shared::AppError;

/// Error type for apalis job boundary.
/// Jobs internally use `AppError`; this wrapper converts at the worker boundary.
#[derive(Debug)]
pub struct WorkerError(pub AppError);

impl std::fmt::Display for WorkerError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    self.0.fmt(f)
  }
}

impl std::error::Error for WorkerError {}

impl From<AppError> for WorkerError {
  fn from(err: AppError) -> Self {
    Self(err)
  }
}

// Enable `?` to automatically convert anyhow errors in job functions
impl From<anyhow::Error> for WorkerError {
  fn from(err: anyhow::Error) -> Self {
    Self(AppError::new(err))
  }
}
