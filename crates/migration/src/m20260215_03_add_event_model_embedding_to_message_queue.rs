use sea_orm_migration::{prelude::*, schema::custom};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
  async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
      .alter_table(
        Table::alter()
          .table(MessageQueue::Table)
          .add_column(custom(MessageQueue::EventModelEmbedding, "vector(1024)").null())
          .to_owned(),
      )
      .await
  }

  async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
      .alter_table(
        Table::alter()
          .table(MessageQueue::Table)
          .drop_column(MessageQueue::EventModelEmbedding)
          .to_owned(),
      )
      .await
  }
}

#[derive(Iden)]
pub enum MessageQueue {
  Table,
  EventModelEmbedding,
}
