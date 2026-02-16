use sea_orm_migration::{
  prelude::*,
  schema::{custom, json_binary, text, uuid},
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
          .col(text(MessageQueue::EventModel).null())
          .col(custom(MessageQueue::LastEmbedding, "vector(1024)").null())
          .col(custom(MessageQueue::EventModelEmbedding, "vector(1024)").null())
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
  // json array of pending reviews
  PendingReviews,
  // last event model content
  EventModel,
  // last embedding vector
  LastEmbedding,
  // event model embedding vector
  EventModelEmbedding,
}
