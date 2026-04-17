use sea_orm_migration::{
  prelude::*,
  schema::{custom, float, json_binary, text, timestamp_with_time_zone, uuid},
  sea_orm::Statement,
};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
  async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
      .create_table(
        Table::create()
          .table(EpisodicMemory::Table)
          .if_not_exists()
          .col(uuid(EpisodicMemory::Id).primary_key())
          .col(uuid(EpisodicMemory::ConversationId).not_null())
          .col(json_binary(EpisodicMemory::Messages).not_null())
          .col(text(EpisodicMemory::Content).not_null())
          .col(custom(EpisodicMemory::Embedding, "vector(1024)").not_null())
          .col(text(EpisodicMemory::Title).not_null())
          .col(float(EpisodicMemory::Stability).not_null())
          .col(float(EpisodicMemory::Difficulty).not_null())
          .col(float(EpisodicMemory::Surprise).not_null())
          .col(text(EpisodicMemory::Classification).null())
          .col(timestamp_with_time_zone(EpisodicMemory::StartAt).not_null())
          .col(timestamp_with_time_zone(EpisodicMemory::EndAt).not_null())
          .col(timestamp_with_time_zone(EpisodicMemory::CreatedAt).not_null())
          .col(timestamp_with_time_zone(EpisodicMemory::LastReviewedAt).not_null())
          .col(timestamp_with_time_zone(EpisodicMemory::ConsolidatedAt).null())
          .to_owned(),
      )
      .await?;

    manager
      .get_connection()
      .execute_raw(Statement::from_string(
        manager.get_database_backend(),
        "ALTER TABLE episodic_memory \
         ADD COLUMN IF NOT EXISTS search_text TEXT \
         GENERATED ALWAYS AS (COALESCE(title, '') || ' ' || COALESCE(content, '')) STORED;",
      ))
      .await?;

    manager
      .get_connection()
      .execute_raw(Statement::from_string(
        manager.get_database_backend(),
        "CREATE INDEX IF NOT EXISTS idx_episodic_memory_embedding_hnsw ON episodic_memory USING hnsw (embedding vector_ip_ops);",
      ))
      .await?;

    manager
      .get_connection()
      .execute_raw(Statement::from_string(
        manager.get_database_backend(),
        "CREATE INDEX IF NOT EXISTS idx_episodic_memory_bm25 ON episodic_memory USING bm25 (id, (search_text::pdb.icu), created_at) WITH (key_field='id');",
      ))
      .await?;

    Ok(())
  }

  async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
      .drop_table(Table::drop().table(EpisodicMemory::Table).to_owned())
      .await
  }
}

#[derive(Iden)]
enum EpisodicMemory {
  Table,
  Id,
  ConversationId,
  Messages,
  Content,
  Embedding,
  Title,
  Stability,
  Difficulty,
  Surprise,
  Classification,
  StartAt,
  EndAt,
  CreatedAt,
  LastReviewedAt,
  ConsolidatedAt,
}
