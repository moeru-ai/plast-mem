use sea_orm_migration::{
  prelude::*,
  schema::{custom, text, timestamp_with_time_zone, uuid},
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
          .table(SemanticMemory::Table)
          .if_not_exists()
          .col(uuid(SemanticMemory::Id).primary_key())
          .col(text(SemanticMemory::Subject).not_null())
          .col(text(SemanticMemory::Predicate).not_null())
          .col(text(SemanticMemory::Object).not_null())
          .col(text(SemanticMemory::Fact).not_null())
          .col(custom(SemanticMemory::SourceEpisodicIds, "UUID[] NOT NULL DEFAULT '{}'"))
          .col(timestamp_with_time_zone(SemanticMemory::ValidAt).not_null().default(Expr::current_timestamp()))
          .col(timestamp_with_time_zone(SemanticMemory::InvalidAt).null())
          .col(custom(SemanticMemory::Embedding, "vector(1024)").not_null())
          .col(timestamp_with_time_zone(SemanticMemory::CreatedAt).not_null().default(Expr::current_timestamp()))
          .to_owned(),
      )
      .await?;

    // HNSW index for vector similarity search on fact embedding
    manager
      .get_connection()
      .execute_raw(Statement::from_string(
        manager.get_database_backend(),
        "CREATE INDEX idx_semantic_memory_embedding ON semantic_memory USING hnsw (embedding vector_ip_ops);",
      ))
      .await?;

    // Partial index for active facts by subject
    manager
      .get_connection()
      .execute_raw(Statement::from_string(
        manager.get_database_backend(),
        "CREATE INDEX idx_semantic_memory_active_subject ON semantic_memory (subject) WHERE invalid_at IS NULL;",
      ))
      .await?;

    // BM25 index for full-text search on fact
    manager
      .get_connection()
      .execute_raw(Statement::from_string(
        manager.get_database_backend(),
        "CREATE INDEX idx_semantic_memory_fact_bm25 ON semantic_memory USING bm25 (id, (fact::pdb.icu), created_at) WITH (key_field='id');",
      ))
      .await?;

    Ok(())
  }

  async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
      .drop_table(Table::drop().table(SemanticMemory::Table).to_owned())
      .await?;

    Ok(())
  }
}

#[derive(Iden)]
pub enum SemanticMemory {
  Table,

  Id,           // uuid v7
  Subject,      // e.g. "user", "assistant", "we"
  Predicate,    // e.g. "likes", "lives_in"
  Object,       // e.g. "Rust", "Tokyo"
  Fact,         // natural language sentence
  SourceEpisodicIds, // source episode IDs (UUID[])
  ValidAt,      // when we learned this fact
  InvalidAt,    // when we learned it was no longer true (NULL = active)
  Embedding,    // vector(1024) embedding of `fact`
  CreatedAt,    // creation timestamp
}
