use sea_orm_migration::{
  prelude::*,
  schema::{big_integer, text, timestamp_with_time_zone, uuid},
};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
  async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
      .create_table(
        Table::create()
          .table(EpisodeSpan::Table)
          .if_not_exists()
          .col(uuid(EpisodeSpan::ConversationId).not_null())
          .col(big_integer(EpisodeSpan::StartSeq).not_null())
          .col(big_integer(EpisodeSpan::EndSeq).not_null())
          .col(text(EpisodeSpan::Classification).not_null())
          .col(
            timestamp_with_time_zone(EpisodeSpan::CreatedAt)
              .not_null()
              .default(Expr::current_timestamp()),
          )
          .primary_key(
            Index::create()
              .col(EpisodeSpan::ConversationId)
              .col(EpisodeSpan::StartSeq),
          )
          .to_owned(),
      )
      .await
  }

  async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
      .drop_table(Table::drop().table(EpisodeSpan::Table).to_owned())
      .await
  }
}

#[derive(Iden)]
enum EpisodeSpan {
  Table,
  ConversationId,
  StartSeq,
  EndSeq,
  Classification,
  CreatedAt,
}
