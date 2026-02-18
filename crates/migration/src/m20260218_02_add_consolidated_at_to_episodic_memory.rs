use sea_orm_migration::{
  prelude::*,
  schema::timestamp_with_time_zone,
};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
  async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
      .alter_table(
        Table::alter()
          .table(EpisodicMemory::Table)
          .add_column(timestamp_with_time_zone(EpisodicMemory::ConsolidatedAt).null())
          .to_owned(),
      )
      .await?;

    Ok(())
  }

  async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
      .alter_table(
        Table::alter()
          .table(EpisodicMemory::Table)
          .drop_column(EpisodicMemory::ConsolidatedAt)
          .to_owned(),
      )
      .await?;

    Ok(())
  }
}

#[derive(Iden)]
pub enum EpisodicMemory {
  Table,
  ConsolidatedAt,
}
