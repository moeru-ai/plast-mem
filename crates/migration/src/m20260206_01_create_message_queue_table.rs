use sea_orm_migration::{
  prelude::*,
  schema::{json_binary, uuid},
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
  // uuid v7, conversation_id
  Id,
  // json messages
  Messages,
}
