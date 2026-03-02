use std::{
  fmt::Display,
  panic::Location,
};

use axum::{
  http::StatusCode,
  response::{IntoResponse, Response},
};
use tracing::error;
use tracing_error::SpanTrace;

#[derive(Debug)]
pub struct AppError {
  err: anyhow::Error,
  status_code: StatusCode,
  span_trace: SpanTrace,
  location: &'static Location<'static>,
}

impl AppError {
  /// Create with 500 status
  #[track_caller]
  pub fn new<E: Into<anyhow::Error>>(err: E) -> Self {
    Self {
      err: err.into(),
      status_code: StatusCode::INTERNAL_SERVER_ERROR,
      span_trace: SpanTrace::capture(),
      location: Location::caller(),
    }
  }

  /// Create with custom status
  #[track_caller]
  pub fn with_status<E: Into<anyhow::Error>>(status: StatusCode, err: E) -> Self {
    Self {
      err: err.into(),
      status_code: status,
      span_trace: SpanTrace::capture(),
      location: Location::caller(),
    }
  }

  #[must_use]
  pub const fn status_code(&self) -> StatusCode {
    self.status_code
  }

  /// Get the captured span trace
  #[must_use]
  pub const fn span_trace(&self) -> &SpanTrace {
    &self.span_trace
  }

  /// Get the source location where the error originated
  #[must_use]
  pub const fn location(&self) -> &'static Location<'static> {
    self.location
  }
}

impl IntoResponse for AppError {
  fn into_response(self) -> Response {
    let body = if cfg!(debug_assertions) {
      format!(
        "{}\n\nError origin: {}:{}\n\nSpan Trace:\n{}",
        self.err,
        self.location.file(),
        self.location.line(),
        self.span_trace
      )
    } else {
      self.err.to_string()
    };

    error!(
      status = %self.status_code,
      error = %self.err,
      location = %format!("{}:{}", self.location.file(), self.location.line()),
      "Request failed"
    );

    (self.status_code, body).into_response()
  }
}

impl Display for AppError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "[{}] {}", self.status_code, self.err)
  }
}

impl<E> From<E> for AppError
where
  E: Into<anyhow::Error>,
{
  #[track_caller]
  fn from(err: E) -> Self {
    Self {
      err: err.into(),
      status_code: StatusCode::INTERNAL_SERVER_ERROR,
      span_trace: SpanTrace::capture(),
      location: Location::caller(),
    }
  }
}
