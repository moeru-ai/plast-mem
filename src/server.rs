use axum::{Router, response::Html, routing::get};
use plast_mem_shared::AppError;
use apalis_postgres::PostgresStorage;
use sea_orm::DatabaseConnection;
use tokio::net::TcpListener;

use crate::{
  api,
  utils::{AppState, shutdown_signal},
};
use plast_mem_worker::WorkerJob;

#[axum::debug_handler]
async fn handler() -> Html<&'static str> {
  Html("<h1>Plast Mem</h1>")
}

pub async fn server(
  db: DatabaseConnection,
  job_storage: PostgresStorage<WorkerJob>,
) -> Result<(), AppError> {
  let app_state = AppState::new(db, job_storage);

  let app = Router::new()
    .route("/", get(handler))
    .merge(api::app())
    .with_state(app_state);

  let listener = TcpListener::bind("0.0.0.0:3000").await?;

  tracing::info!("server started at http://0.0.0.0:3000");

  axum::serve(listener, app)
    .with_graceful_shutdown(shutdown_signal())
    .await?;

  Ok(())
}
