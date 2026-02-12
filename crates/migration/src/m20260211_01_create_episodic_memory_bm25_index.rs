use sea_orm_migration::{prelude::*, sea_orm::Statement};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
  async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
      .get_connection()
      .execute_raw(Statement::from_string(
        manager.get_database_backend(),
        "CREATE INDEX bm25_index ON episodic_memory USING bm25 (id, (content::pdb.icu), created_at) WITH (key_field='id');",
      ))
      .await?;

    Ok(())
  }

  async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
      .get_connection()
      .execute_raw(Statement::from_string(
        manager.get_database_backend(),
        "DROP INDEX IF EXISTS bm25_index;",
      ))
      .await?;

    Ok(())
  }
}
