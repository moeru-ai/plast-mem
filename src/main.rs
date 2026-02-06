use std::env;

use apalis_postgres::PostgresStorage;
use plast_mem_db_migration::{Migrator, MigratorTrait};
use sea_orm::Database;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod api;
mod core;
mod server;
mod utils;
mod worker;

use utils::AppError;

use crate::server::server;
use crate::worker::worker;

#[tokio::main]
async fn main() -> Result<(), AppError> {
  tracing_subscriber::registry()
    .with(
      tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| format!("{}=debug", env!("CARGO_CRATE_NAME")).into()),
    )
    .with(tracing_subscriber::fmt::layer())
    .init();
  dotenvy::dotenv().ok();

  let db = Database::connect(
    env::var("DATABASE_URL") // TODO: unwrap_or_else
      .expect("plast-mem: invalid database url")
      .as_str(),
  )
  .await?;

  // Apply all pending migrations
  // https://www.sea-ql.org/SeaORM/docs/migration/running-migration/#migrating-programmatically
  Migrator::up(&db, None).await?;
  PostgresStorage::setup(&db.get_postgres_connection_pool()).await?;

  let _ = tokio::try_join!(worker(&db), server(db.clone()));

  Ok(())
}
