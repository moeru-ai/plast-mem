use apalis_postgres::PostgresStorage;
use plastmem_migration::{Migrator, MigratorTrait};
use plastmem_server::server;
use plastmem_shared::{APP_ENV, AppError};
use plastmem_worker::{EventSegmentationJob, MemoryReviewJob, SemanticConsolidationJob, worker};
use sea_orm::Database;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<(), AppError> {
  tracing_subscriber::registry()
    .with(
      tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| format!("{}=debug", env!("CARGO_CRATE_NAME")).into()),
    )
    .with(tracing_subscriber::fmt::layer())
    .init();

  let db = Database::connect(APP_ENV.database_url.as_str()).await?;

  // Apply all pending migrations
  // https://www.sea-ql.org/SeaORM/docs/migration/running-migration/#migrating-programmatically
  Migrator::up(&db, None).await?;
  let pool = db.get_postgres_connection_pool();
  PostgresStorage::setup(pool).await?;
  let segment_job_storage = PostgresStorage::<EventSegmentationJob>::new(pool);
  let review_job_storage = PostgresStorage::<MemoryReviewJob>::new(pool);
  let semantic_job_storage = PostgresStorage::<SemanticConsolidationJob>::new(pool);

  let _ = tokio::try_join!(
    worker(&db, segment_job_storage.clone(), review_job_storage.clone(), semantic_job_storage.clone()),
    server(db.clone(), segment_job_storage)
  );

  Ok(())
}
