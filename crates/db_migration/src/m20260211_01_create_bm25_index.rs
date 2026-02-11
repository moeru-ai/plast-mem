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
        r#"CALL paradedb.create_bm25(
          index_name => 'bm25_index',
          table_name => 'episodic_memory',
          key_field => 'id',
          text_fields => paradedb.field('content')
        );"#,
      ))
      .await?;

    Ok(())
  }

  async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
      .get_connection()
      .execute_raw(Statement::from_string(
        manager.get_database_backend(),
        "CALL paradedb.drop_bm25(index_name => 'bm25_index');",
      ))
      .await?;

    Ok(())
  }
}
