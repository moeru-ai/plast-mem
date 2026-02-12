use sea_orm_migration::{
  prelude::*,
  schema::{custom, json_binary, string, timestamp_with_time_zone, uuid},
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
          .col(string(EpisodicMemory::Content))
          .col(custom(EpisodicMemory::Embedding, "vector(1024)").not_null())
          .col(timestamp_with_time_zone(EpisodicMemory::StartAt))
          .col(timestamp_with_time_zone(EpisodicMemory::EndAt))
          .col(timestamp_with_time_zone(EpisodicMemory::CreatedAt))
          .col(timestamp_with_time_zone(EpisodicMemory::LastReviewedAt))
          .to_owned(),
      )
      .await?;

    manager
      .get_connection()
      .execute_raw(Statement::from_string(
        manager.get_database_backend(),
        "CREATE INDEX cosine_index ON episodic_memory USING hnsw (embedding vector_cosine_ops);",
      ))
      .await?;

    Ok(())
  }

  async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    // manager
    //   .get_connection()
    //   .execute_raw(Statement::from_string(
    //     manager.get_database_backend(),
    //     "DROP INDEX IF EXISTS cosine_index;",
    //   ))
    //   .await?;

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
  // TODO: PreviousId
  // TODO: RelatedIds

  // json messages
  Messages,
  // formatted messages (for bm25)
  Content,
  // formatted messages embedding (for cosine similarity)
  Embedding,

  // FSRS Memory State
  Stability,
  Difficulty,

  // earliest message timestamp
  StartAt,
  // latest message timestamp
  EndAt,
  // create timestamp
  CreatedAt,
  // last review timestamp
  LastReviewedAt,
}
