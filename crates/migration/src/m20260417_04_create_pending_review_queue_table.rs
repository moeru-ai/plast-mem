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
          .table(PendingReviewQueue::Table)
          .if_not_exists()
          .col(uuid(PendingReviewQueue::Id).primary_key())
          .col(uuid(PendingReviewQueue::ConversationId).not_null())
          .col(text(PendingReviewQueue::Query).not_null())
          .col(custom(
            PendingReviewQueue::MemoryIds,
            "UUID[] NOT NULL DEFAULT '{}'",
          ))
          .col(
            timestamp_with_time_zone(PendingReviewQueue::CreatedAt)
              .not_null()
              .default(Expr::current_timestamp()),
          )
          .col(timestamp_with_time_zone(PendingReviewQueue::ConsumedAt).null())
          .to_owned(),
      )
      .await?;

    manager
      .get_connection()
      .execute_raw(Statement::from_string(
        manager.get_database_backend(),
        "CREATE INDEX IF NOT EXISTS idx_pending_review_queue_conversation_unconsumed ON pending_review_queue (conversation_id, created_at) WHERE consumed_at IS NULL;",
      ))
      .await?;

    Ok(())
  }

  async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
      .drop_table(Table::drop().table(PendingReviewQueue::Table).to_owned())
      .await
  }
}

#[derive(Iden)]
enum PendingReviewQueue {
  Table,
  Id,
  ConversationId,
  Query,
  MemoryIds,
  CreatedAt,
  ConsumedAt,
}
