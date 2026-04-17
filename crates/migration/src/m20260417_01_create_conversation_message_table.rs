use sea_orm_migration::{
  prelude::*,
  schema::{big_integer, text, timestamp_with_time_zone, uuid},
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
          .table(ConversationMessage::Table)
          .if_not_exists()
          .col(uuid(ConversationMessage::ConversationId).not_null())
          .col(big_integer(ConversationMessage::Seq).not_null())
          .col(text(ConversationMessage::Role).not_null())
          .col(text(ConversationMessage::Content).not_null())
          .col(timestamp_with_time_zone(ConversationMessage::Timestamp).not_null())
          .primary_key(
            Index::create()
              .col(ConversationMessage::ConversationId)
              .col(ConversationMessage::Seq),
          )
          .to_owned(),
      )
      .await?;

    manager
      .get_connection()
      .execute_raw(Statement::from_string(
        manager.get_database_backend(),
        "CREATE INDEX IF NOT EXISTS idx_conversation_message_timestamp ON conversation_message (conversation_id, timestamp);",
      ))
      .await?;

    Ok(())
  }

  async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
      .drop_table(Table::drop().table(ConversationMessage::Table).to_owned())
      .await
  }
}

#[derive(Iden)]
enum ConversationMessage {
  Table,
  ConversationId,
  Seq,
  Role,
  Content,
  Timestamp,
}
