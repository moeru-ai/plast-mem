use std::fmt::Display;

use axum::{
  http::StatusCode,
  response::{IntoResponse, Response},
};

// TODO: https://github.com/launchbadge/realworld-axum-sqlx/blob/main/src/http/error.rs
#[derive(Debug)]
pub struct AppError(anyhow::Error);

impl IntoResponse for AppError {
  fn into_response(self) -> Response {
    (
      StatusCode::INTERNAL_SERVER_ERROR,
      format!("Something went wrong: {}", self.0),
    )
      .into_response()
  }
}

impl Display for AppError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    self.0.fmt(f)
  }
}

impl<E> From<E> for AppError
where
  E: Into<anyhow::Error>,
{
  fn from(err: E) -> Self {
    Self(err.into())
  }
}
