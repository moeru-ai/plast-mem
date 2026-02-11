mod event_segmentation;
pub use event_segmentation::*;
use plast_mem_shared::AppError;

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
