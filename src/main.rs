#[cfg(debug_assertions)]
use apalis_board_api::sse::{TracingBroadcaster, TracingSubscriber};
use apalis_postgres::PostgresStorage;
use plastmem_migration::{Migrator, MigratorTrait};
use plastmem_server::server;
use plastmem_shared::{APP_ENV, AppError};
use plastmem_worker::{
  EpisodeCreationJob, EventSegmentationJob, MemoryReviewJob, PredictCalibrateJob, worker,
};
use sea_orm::Database;
use tracing_error::ErrorLayer;
#[cfg(debug_assertions)]
use tracing_subscriber::layer::Layer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<(), AppError> {
  #[cfg(debug_assertions)]
  let board_broadcaster = TracingBroadcaster::create();
  #[cfg(debug_assertions)]
  let board_tracing = TracingSubscriber::new(&board_broadcaster);

  let subscriber = tracing_subscriber::registry()
    .with(
      tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| format!("{}=debug", env!("CARGO_CRATE_NAME")).into()),
    )
    .with(tracing_subscriber::fmt::layer())
    .with(ErrorLayer::default());

  #[cfg(debug_assertions)]
  let subscriber = subscriber.with(
    board_tracing.layer().with_filter(
      tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| format!("{}=debug", env!("CARGO_CRATE_NAME")).into()),
    ),
  );

  subscriber.init();

  let db = Database::connect(APP_ENV.database_url.as_str()).await?;

  // Apply all pending migrations
  // https://www.sea-ql.org/SeaORM/docs/migration/running-migration/#migrating-programmatically
  Migrator::up(&db, None).await?;
  let pool = db.get_postgres_connection_pool();
  PostgresStorage::setup(pool).await?;
  let segment_job_storage = PostgresStorage::<EventSegmentationJob>::new(pool);
  let episode_creation_job_storage = PostgresStorage::<EpisodeCreationJob>::new(pool);
  let review_job_storage = PostgresStorage::<MemoryReviewJob>::new(pool);
  let semantic_job_storage = PostgresStorage::<PredictCalibrateJob>::new(pool);

  let _ = tokio::try_join!(
    worker(
      &db,
      segment_job_storage.clone(),
      episode_creation_job_storage.clone(),
      review_job_storage.clone(),
      semantic_job_storage.clone()
    ),
    server(
      db.clone(),
      segment_job_storage,
      episode_creation_job_storage,
      review_job_storage,
      semantic_job_storage,
      #[cfg(debug_assertions)]
      board_broadcaster
    )
  )?;

  Ok(())
}
