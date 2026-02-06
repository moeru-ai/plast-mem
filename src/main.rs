use axum::{Router, response::Html, routing::get};
use tokio::net::TcpListener;

mod api;
mod core;
mod services;
mod state;
mod utils;

use state::AppState;
use utils::{AppError, shutdown_signal};

#[axum::debug_handler]
async fn handler() -> Html<&'static str> {
  Html("<h1>Plast Mem</h1>")
}

#[tokio::main]
async fn main() -> Result<(), AppError> {
  tracing_subscriber::fmt::init();

  let app_state = AppState::new();

  let app = Router::new()
    .route("/", get(handler))
    .merge(api::app())
    .with_state(app_state);
  let listener = TcpListener::bind("0.0.0.0:3000").await?;

  tracing::info!("server started at http://0.0.0.0:3000");

  Ok(
    axum::serve(listener, app)
      .with_graceful_shutdown(shutdown_signal())
      .await?,
  )
}
