use sea_orm_migration::{prelude::*, sea_orm::Statement};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
  async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    let conn = manager.get_connection();
    let backend = manager.get_database_backend();

    // IMMUTABLE wrapper — array_to_string is STABLE in PostgreSQL, which is not
    // allowed in a generated column expression. Wrapping it in an explicitly
    // IMMUTABLE function is the standard workaround.
    conn
      .execute_raw(Statement::from_string(
        backend,
        "CREATE OR REPLACE FUNCTION immutable_keywords_to_text(TEXT[]) RETURNS TEXT \
         LANGUAGE sql IMMUTABLE STRICT AS $$ SELECT array_to_string($1, ' ') $$;",
      ))
      .await?;

    // Drop old BM25 index (will recreate to cover keywords)
    conn
      .execute_raw(Statement::from_string(
        backend,
        "DROP INDEX IF EXISTS idx_semantic_memory_fact_bm25;",
      ))
      .await?;

    // Drop partial index on subject (column being removed)
    conn
      .execute_raw(Statement::from_string(
        backend,
        "DROP INDEX IF EXISTS idx_semantic_memory_active_subject;",
      ))
      .await?;

    // Remove SPO columns (IF EXISTS — safe to re-run if migration was partial)
    conn
      .execute_raw(Statement::from_string(
        backend,
        "ALTER TABLE semantic_memory \
         DROP COLUMN IF EXISTS subject, \
         DROP COLUMN IF EXISTS predicate, \
         DROP COLUMN IF EXISTS object;",
      ))
      .await?;

    // Add category and keywords columns (IF NOT EXISTS — idempotent)
    conn
      .execute_raw(Statement::from_string(
        backend,
        "ALTER TABLE semantic_memory \
         ADD COLUMN IF NOT EXISTS category TEXT NOT NULL DEFAULT 'identity', \
         ADD COLUMN IF NOT EXISTS keywords TEXT[] NOT NULL DEFAULT '{}';",
      ))
      .await?;

    // Add generated search_text column for BM25 indexing (IF NOT EXISTS — idempotent)
    conn
      .execute_raw(Statement::from_string(
        backend,
        "ALTER TABLE semantic_memory \
         ADD COLUMN IF NOT EXISTS search_text TEXT \
         GENERATED ALWAYS AS (fact || ' ' || immutable_keywords_to_text(keywords)) STORED;",
      ))
      .await?;

    // Recreate BM25 index on search_text
    conn
      .execute_raw(Statement::from_string(
        backend,
        "CREATE INDEX IF NOT EXISTS idx_semantic_memory_bm25 ON semantic_memory \
         USING bm25 (id, (search_text::pdb.icu), created_at) WITH (key_field='id');",
      ))
      .await?;

    // Partial index on category for active facts
    conn
      .execute_raw(Statement::from_string(
        backend,
        "CREATE INDEX IF NOT EXISTS idx_semantic_memory_active_category \
         ON semantic_memory (category) WHERE invalid_at IS NULL;",
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
        "DROP INDEX IF EXISTS idx_semantic_memory_bm25;",
      ))
      .await?;

    conn
      .execute_raw(Statement::from_string(
        backend,
        "DROP INDEX IF EXISTS idx_semantic_memory_active_category;",
      ))
      .await?;

    conn
      .execute_raw(Statement::from_string(
        backend,
        "ALTER TABLE semantic_memory \
         DROP COLUMN IF EXISTS search_text, \
         DROP COLUMN IF EXISTS category, \
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
        "ALTER TABLE semantic_memory \
         ADD COLUMN IF NOT EXISTS subject TEXT NOT NULL DEFAULT '', \
         ADD COLUMN IF NOT EXISTS predicate TEXT NOT NULL DEFAULT '', \
         ADD COLUMN IF NOT EXISTS object TEXT NOT NULL DEFAULT '';",
      ))
      .await?;

    conn
      .execute_raw(Statement::from_string(
        backend,
        "CREATE INDEX IF NOT EXISTS idx_semantic_memory_fact_bm25 ON semantic_memory \
         USING bm25 (id, (fact::pdb.icu), created_at) WITH (key_field='id');",
      ))
      .await?;

    conn
      .execute_raw(Statement::from_string(
        backend,
        "CREATE INDEX IF NOT EXISTS idx_semantic_memory_active_subject \
         ON semantic_memory (subject) WHERE invalid_at IS NULL;",
      ))
      .await?;

    Ok(())
  }
}
