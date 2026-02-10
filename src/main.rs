use apalis_postgres::PostgresStorage;
use plast_mem_db_migration::{Migrator, MigratorTrait};
use plast_mem_shared::{APP_ENV, AppError};
use plast_mem_worker::{WorkerJob, worker};
use sea_orm::Database;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod api;
mod server;
mod utils;

use crate::server::server;

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
  PostgresStorage::setup(&db.get_postgres_connection_pool()).await?;
  let job_storage = PostgresStorage::<WorkerJob>::new(db.get_postgres_connection_pool());

  let _ = tokio::try_join!(
    worker(&db, job_storage.clone()),
    server(db.clone(), job_storage)
  );

  Ok(())
}
