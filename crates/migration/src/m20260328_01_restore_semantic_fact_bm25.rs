use sea_orm_migration::{prelude::*, sea_orm::Statement};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
  async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    let conn = manager.get_connection();
    let backend = manager.get_database_backend();

    conn
      .execute_raw(Statement::from_string(
        backend,
        "DROP INDEX IF EXISTS idx_semantic_memory_bm25;",
      ))
      .await?;

    conn
      .execute_raw(Statement::from_string(
        backend,
        "ALTER TABLE semantic_memory \
         DROP COLUMN IF EXISTS search_text, \
         DROP COLUMN IF EXISTS keywords;",
      ))
      .await?;

    conn
      .execute_raw(Statement::from_string(
        backend,
        "DROP FUNCTION IF EXISTS immutable_keywords_to_text(TEXT[]);",
      ))
      .await?;

    conn
      .execute_raw(Statement::from_string(
        backend,
        "CREATE INDEX IF NOT EXISTS idx_semantic_memory_fact_bm25 ON semantic_memory \
         USING bm25 (id, (fact::pdb.icu), created_at) WITH (key_field='id');",
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
        "DROP INDEX IF EXISTS idx_semantic_memory_fact_bm25;",
      ))
      .await?;

    conn
      .execute_raw(Statement::from_string(
        backend,
        "CREATE OR REPLACE FUNCTION immutable_keywords_to_text(TEXT[]) RETURNS TEXT \
         LANGUAGE sql IMMUTABLE STRICT AS $$ SELECT array_to_string($1, ' ') $$;",
      ))
      .await?;

    conn
      .execute_raw(Statement::from_string(
        backend,
        "ALTER TABLE semantic_memory \
         ADD COLUMN IF NOT EXISTS keywords TEXT[] NOT NULL DEFAULT '{}';",
      ))
      .await?;

    conn
      .execute_raw(Statement::from_string(
        backend,
        "ALTER TABLE semantic_memory \
         ADD COLUMN IF NOT EXISTS search_text TEXT \
         GENERATED ALWAYS AS (fact || ' ' || immutable_keywords_to_text(keywords)) STORED;",
      ))
      .await?;

    conn
      .execute_raw(Statement::from_string(
        backend,
        "CREATE INDEX IF NOT EXISTS idx_semantic_memory_bm25 ON semantic_memory \
         USING bm25 (id, (search_text::pdb.icu), created_at) WITH (key_field='id');",
      ))
      .await?;

    Ok(())
  }
}
