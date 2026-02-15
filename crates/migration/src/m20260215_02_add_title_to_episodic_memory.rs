use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
  async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
      .alter_table(
        Table::alter()
          .table(EpisodicMemory::Table)
          .add_column(
            ColumnDef::new(EpisodicMemory::Title)
              .text()
              .not_null()
              .default(""),
          )
          .to_owned(),
      )
      .await
  }

  async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
      .alter_table(
        Table::alter()
          .table(EpisodicMemory::Table)
          .drop_column(EpisodicMemory::Title)
          .to_owned(),
      )
      .await
  }
}

#[derive(Iden)]
pub enum EpisodicMemory {
  Table,
  Title,
}
