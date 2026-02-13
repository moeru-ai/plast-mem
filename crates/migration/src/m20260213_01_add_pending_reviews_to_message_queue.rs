use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
  async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
      .alter_table(
        Table::alter()
          .table(MessageQueue::Table)
          .add_column(
            ColumnDef::new(MessageQueue::PendingReviews)
              .json_binary()
              .null(),
          )
          .to_owned(),
      )
      .await
  }

  async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
      .alter_table(
        Table::alter()
          .table(MessageQueue::Table)
          .drop_column(MessageQueue::PendingReviews)
          .to_owned(),
      )
      .await
  }
}

#[derive(Iden)]
pub enum MessageQueue {
  Table,
  PendingReviews,
}
