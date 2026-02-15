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
          .add_column(ColumnDef::new(MessageQueue::EventModel).text().null())
          .add_column(custom(MessageQueue::LastEmbedding, "vector(1024)").null())
          .to_owned(),
      )
      .await
  }

  async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
      .alter_table(
        Table::alter()
          .table(MessageQueue::Table)
          .drop_column(MessageQueue::EventModel)
          .drop_column(MessageQueue::LastEmbedding)
          .to_owned(),
      )
      .await
  }
}

#[derive(Iden)]
pub enum MessageQueue {
  Table,
  EventModel,
  LastEmbedding,
}
