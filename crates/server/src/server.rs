#[cfg(debug_assertions)]
use apalis_board_api::{
  framework::{ApiBuilder, RegisterRoute},
  sse::TracingBroadcaster,
  ui::ServeUI,
};
use apalis_postgres::PostgresStorage;
#[cfg(not(debug_assertions))]
use axum::response::Html;
use axum::{Router, routing::get};
#[cfg(debug_assertions)]
use axum::{Extension, response::Redirect};
use plastmem_shared::AppError;
use plastmem_worker::{EventSegmentationJob, MemoryReviewJob, PredictCalibrateJob};
use sea_orm::DatabaseConnection;
#[cfg(debug_assertions)]
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;

use crate::{
  api,
  utils::{AppState, shutdown_signal},
};

#[cfg(not(debug_assertions))]
#[axum::debug_handler]
#[tracing::instrument]
async fn handler() -> Html<&'static str> {
  Html("<h1>Plast Mem</h1>")
}

pub async fn server(
  db: DatabaseConnection,
  segment_job_storage: PostgresStorage<EventSegmentationJob>,
  review_job_storage: PostgresStorage<MemoryReviewJob>,
  predict_calibrate_job_storage: PostgresStorage<PredictCalibrateJob>,
  #[cfg(debug_assertions)] board_broadcaster: Arc<Mutex<TracingBroadcaster>>,
) -> Result<(), AppError> {
  let app_state = AppState::new(
    db,
    segment_job_storage,
    review_job_storage,
    predict_calibrate_job_storage,
  );

  let app = Router::new().merge(api::app());
  #[cfg(not(debug_assertions))]
  let app = app.route("/", get(handler));
  let app = app
    .merge(board_app(
      &app_state,
      #[cfg(debug_assertions)]
      board_broadcaster,
    ))
    .with_state(app_state);

  let listener = TcpListener::bind("0.0.0.0:3000").await?;

  tracing::info!("server started at http://0.0.0.0:3000");

  axum::serve(listener, app)
    .with_graceful_shutdown(shutdown_signal())
    .await?;

  Ok(())
}

#[cfg(debug_assertions)]
fn board_app(
  app_state: &AppState,
  board_broadcaster: Arc<Mutex<TracingBroadcaster>>,
) -> Router<AppState> {
  let board_api = ApiBuilder::new(Router::new())
    .register(app_state.segmentation_job_storage.clone())
    .register(app_state.review_job_storage.clone())
    .register(app_state.predict_calibrate_job_storage.clone())
    .build();

  let board = Router::new()
    .route("/board", get(|| async { Redirect::permanent("/") }))
    .route("/board/", get(|| async { Redirect::permanent("/") }))
    .nest("/api/v1", board_api)
    .fallback_service(ServeUI::new())
    .layer(Extension(board_broadcaster));

  board.with_state::<AppState>(())
}

#[cfg(not(debug_assertions))]
fn board_app(_app_state: &AppState) -> Router<AppState> {
  Router::<AppState>::new()
}
