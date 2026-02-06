use sea_orm::DatabaseConnection;

#[derive(Clone, Debug, Default)]
pub struct AppState {
  pub db: DatabaseConnection,
}

impl AppState {
  pub fn new(db: DatabaseConnection) -> Self {
    Self { db }
  }
}
