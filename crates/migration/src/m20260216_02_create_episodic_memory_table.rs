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
          .col(uuid(EpisodicMemory::ConversationId))
          .col(json_binary(EpisodicMemory::Messages))
          .col(text(EpisodicMemory::Summary))
          .col(custom(EpisodicMemory::Embedding, "vector(1024)").not_null())
          .col(text(EpisodicMemory::Title).not_null().default(""))
          // FSRS Memory State
          .col(float(EpisodicMemory::Stability))
          .col(float(EpisodicMemory::Difficulty))
          // Surprise for initial stability and display
          .col(float(EpisodicMemory::Surprise))
          // Timestamps
          .col(timestamp_with_time_zone(EpisodicMemory::StartAt))
          .col(timestamp_with_time_zone(EpisodicMemory::EndAt))
          .col(timestamp_with_time_zone(EpisodicMemory::CreatedAt))
          .col(timestamp_with_time_zone(EpisodicMemory::LastReviewedAt))
          .to_owned(),
      )
      .await?;

    // HNSW index for vector similarity search
    manager
      .get_connection()
      .execute_raw(Statement::from_string(
        manager.get_database_backend(),
        "CREATE INDEX idx_episodic_memory_embedding_hnsw ON episodic_memory USING hnsw (embedding vector_cosine_ops);",
      ))
      .await?;

    // BM25 index for full-text search
    manager
      .get_connection()
      .execute_raw(Statement::from_string(
        manager.get_database_backend(),
        "CREATE INDEX idx_episodic_memory_summary_bm25 ON episodic_memory USING bm25 (id, (summary::pdb.icu), created_at) WITH (key_field='id');",
      ))
      .await?;

    Ok(())
  }

  async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
      .drop_table(Table::drop().table(EpisodicMemory::Table).to_owned())
      .await?;

    Ok(())
  }
}

#[derive(Iden)]
pub enum EpisodicMemory {
  Table,

  Id,             // uuid v7
  ConversationId, // uuid v7

  // json messages
  Messages,
  // memory summary (for bm25)
  Summary,
  // memory summary embedding (for cosine similarity)
  Embedding,
  // memory title
  Title,

  // FSRS Memory State
  Stability,
  Difficulty,

  // Surprise for initial stability and display
  Surprise,

  // earliest message timestamp
  StartAt,
  // latest message timestamp
  EndAt,
  // create timestamp
  CreatedAt,
  // last review timestamp
  LastReviewedAt,
}
