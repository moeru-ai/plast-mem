use apalis_postgres::PostgresStorage;
use axum::{Router, response::Html, routing::get};
use plastmem_shared::AppError;
use plastmem_worker::{EventSegmentationJob, MemoryReviewJob};
use sea_orm::DatabaseConnection;
use tokio::net::TcpListener;

use crate::{
  api,
  utils::{AppState, shutdown_signal},
};

#[axum::debug_handler]
async fn handler() -> Html<&'static str> {
  Html("<h1>Plast Mem</h1>")
}

pub async fn server(
  db: DatabaseConnection,
  segment_job_storage: PostgresStorage<EventSegmentationJob>,
  review_job_storage: PostgresStorage<MemoryReviewJob>,
) -> Result<(), AppError> {
  let app_state = AppState::new(db, segment_job_storage, review_job_storage);

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
