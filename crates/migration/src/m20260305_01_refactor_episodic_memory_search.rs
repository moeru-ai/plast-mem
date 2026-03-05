use sea_orm_migration::{prelude::*, sea_orm::Statement};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
  async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    let conn = manager.get_connection();
    let backend = manager.get_database_backend();

    // Drop old BM25 index on summary only.
    conn
      .execute_raw(Statement::from_string(
        backend,
        "DROP INDEX IF EXISTS idx_episodic_memory_summary_bm25;",
      ))
      .await?;

    // Add generated search_text for title + summary + raw messages.
    conn
      .execute_raw(Statement::from_string(
        backend,
        "ALTER TABLE episodic_memory \
         ADD COLUMN IF NOT EXISTS search_text TEXT \
         GENERATED ALWAYS AS (\
           COALESCE(title, '') || ' ' || COALESCE(summary, '') || ' ' || COALESCE(messages::text, '')\
         ) STORED;",
      ))
      .await?;

    // Recreate BM25 index on search_text.
    conn
      .execute_raw(Statement::from_string(
        backend,
        "CREATE INDEX IF NOT EXISTS idx_episodic_memory_bm25 ON episodic_memory \
         USING bm25 (id, (search_text::pdb.icu), created_at) WITH (key_field='id');",
      ))
      .await?;

    Ok(())
  }

  async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    let conn = manager.get_connection();
    let backend = manager.get_database_backend();

    conn
      .execute_raw(Statement::from_string(
        backend,
        "DROP INDEX IF EXISTS idx_episodic_memory_bm25;",
      ))
      .await?;

    conn
      .execute_raw(Statement::from_string(
        backend,
        "ALTER TABLE episodic_memory DROP COLUMN IF EXISTS search_text;",
      ))
      .await?;

    conn
      .execute_raw(Statement::from_string(
        backend,
        "CREATE INDEX IF NOT EXISTS idx_episodic_memory_summary_bm25 ON episodic_memory \
         USING bm25 (id, (summary::pdb.icu), created_at) WITH (key_field='id');",
      ))
      .await?;

    Ok(())
  }
}
