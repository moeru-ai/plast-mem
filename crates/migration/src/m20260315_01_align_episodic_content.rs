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
        r#"
        DO $$
        BEGIN
          IF EXISTS (
            SELECT 1
            FROM information_schema.columns
            WHERE table_name = 'episodic_memory' AND column_name = 'summary'
          ) THEN
            ALTER TABLE episodic_memory RENAME COLUMN summary TO content;
          END IF;
        END $$;
        "#,
      ))
      .await?;

    conn
      .execute_raw(Statement::from_string(
        backend,
        r#"
        DO $$
        BEGIN
          IF EXISTS (
            SELECT 1
            FROM information_schema.columns
            WHERE table_name = 'message_queue' AND column_name = 'prev_episode_summary'
          ) THEN
            ALTER TABLE message_queue RENAME COLUMN prev_episode_summary TO prev_episode_content;
          END IF;
        END $$;
        "#,
      ))
      .await?;

    conn
      .execute_raw(Statement::from_string(
        backend,
        "ALTER TABLE episodic_memory \
         ADD COLUMN IF NOT EXISTS search_text TEXT \
         GENERATED ALWAYS AS (\
           COALESCE(title, '') || ' ' || COALESCE(content, '') || ' ' || COALESCE(messages::text, '')\
         ) STORED;",
      ))
      .await?;

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
        r#"
        DO $$
        BEGIN
          IF EXISTS (
            SELECT 1
            FROM information_schema.columns
            WHERE table_name = 'episodic_memory' AND column_name = 'content'
          ) THEN
            ALTER TABLE episodic_memory RENAME COLUMN content TO summary;
          END IF;
        END $$;
        "#,
      ))
      .await?;

    conn
      .execute_raw(Statement::from_string(
        backend,
        r#"
        DO $$
        BEGIN
          IF EXISTS (
            SELECT 1
            FROM information_schema.columns
            WHERE table_name = 'message_queue' AND column_name = 'prev_episode_content'
          ) THEN
            ALTER TABLE message_queue RENAME COLUMN prev_episode_content TO prev_episode_summary;
          END IF;
        END $$;
        "#,
      ))
      .await?;

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

    conn
      .execute_raw(Statement::from_string(
        backend,
        "CREATE INDEX IF NOT EXISTS idx_episodic_memory_bm25 ON episodic_memory \
         USING bm25 (id, (search_text::pdb.icu), created_at) WITH (key_field='id');",
      ))
      .await?;

    Ok(())
  }
}
