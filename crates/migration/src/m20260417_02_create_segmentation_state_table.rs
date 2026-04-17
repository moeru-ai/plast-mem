use sea_orm_migration::{
  prelude::*,
  schema::{big_integer, boolean, timestamp_with_time_zone, uuid},
};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
  async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
      .create_table(
        Table::create()
          .table(SegmentationState::Table)
          .if_not_exists()
          .col(uuid(SegmentationState::ConversationId).primary_key())
          .col(
            big_integer(SegmentationState::LastMessageSeq)
              .not_null()
              .default(-1),
          )
          .col(
            boolean(SegmentationState::EofIdentified)
              .not_null()
              .default(false),
          )
          .col(
            big_integer(SegmentationState::NextSegmentStartSeq)
              .not_null()
              .default(0),
          )
          .col(big_integer(SegmentationState::ActiveSegmentStartSeq).null())
          .col(big_integer(SegmentationState::ActiveSegmentEndSeq).null())
          .col(timestamp_with_time_zone(SegmentationState::ActiveSince).null())
          .to_owned(),
      )
      .await
  }

  async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
      .drop_table(Table::drop().table(SegmentationState::Table).to_owned())
      .await
  }
}

#[derive(Iden)]
enum SegmentationState {
  Table,
  ConversationId,
  LastMessageSeq,
  EofIdentified,
  NextSegmentStartSeq,
  ActiveSegmentStartSeq,
  ActiveSegmentEndSeq,
  ActiveSince,
}
