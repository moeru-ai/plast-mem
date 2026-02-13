use std::{
  backtrace::{Backtrace, BacktraceStatus},
  fmt::Display,
};

use axum::{
  http::StatusCode,
  response::{IntoResponse, Response},
};

#[derive(Debug)]
pub struct AppError {
  err: anyhow::Error,
  status_code: StatusCode,
}

impl AppError {
  /// Create with 500 status
  pub fn new<E: Into<anyhow::Error>>(err: E) -> Self {
    Self {
      err: err.into(),
      status_code: StatusCode::INTERNAL_SERVER_ERROR,
    }
  }

  /// Create with custom status
  pub fn with_status<E: Into<anyhow::Error>>(status: StatusCode, err: E) -> Self {
    Self {
      err: err.into(),
      status_code: status,
    }
  }

  #[must_use]
  pub const fn status_code(&self) -> StatusCode {
    self.status_code
  }

  /// Get backtrace from anyhow (requires `RUST_BACKTRACE=1` to capture)
  pub fn backtrace(&self) -> &Backtrace {
    self.err.backtrace()
  }
}

impl IntoResponse for AppError {
  fn into_response(self) -> Response {
    let body = if cfg!(debug_assertions) {
      let bt = self.err.backtrace();
      if bt.status() == BacktraceStatus::Captured {
        format!("{}\nBacktrace:\n{}", self.err, bt)
      } else {
        format!(
          "{}\n(hint: set RUST_BACKTRACE=1 to enable backtrace)",
          self.err
        )
      }
    } else {
      self.err.to_string()
    };
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
  fn from(err: E) -> Self {
    Self::new(err)
  }
}
