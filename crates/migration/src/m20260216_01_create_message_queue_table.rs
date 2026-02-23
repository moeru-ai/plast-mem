use sea_orm_migration::{
  prelude::*,
  schema::{boolean, integer, json_binary, text, timestamp_with_time_zone, uuid},
};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
  async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
      .create_table(
        Table::create()
          .table(MessageQueue::Table)
          .if_not_exists()
          .col(uuid(MessageQueue::Id).primary_key())
          .col(json_binary(MessageQueue::Messages))
          .col(json_binary(MessageQueue::PendingReviews).null())
          .col(integer(MessageQueue::InProgressFence).null())
          .col(timestamp_with_time_zone(MessageQueue::InProgressSince).null())
          .col(boolean(MessageQueue::WindowDoubled).default(false).not_null())
          .col(text(MessageQueue::PrevEpisodeSummary).null())
          .to_owned(),
      )
      .await
  }

  async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
      .drop_table(Table::drop().table(MessageQueue::Table).to_owned())
      .await
  }
}

#[derive(Iden)]
pub enum MessageQueue {
  Table,
  Id,
  Messages,
  PendingReviews,
  InProgressFence,
  InProgressSince,
  WindowDoubled,
  PrevEpisodeSummary,
}
