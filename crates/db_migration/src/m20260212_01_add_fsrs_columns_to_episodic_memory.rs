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
            ColumnDef::new(EpisodicMemory::Stability)
              .float()
              .not_null()
              .default(0.4),
          )
          .add_column(
            ColumnDef::new(EpisodicMemory::Difficulty)
              .float()
              .not_null()
              .default(5.0),
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
          .drop_column(EpisodicMemory::Stability)
          .drop_column(EpisodicMemory::Difficulty)
          .to_owned(),
      )
      .await
  }
}

#[derive(Iden)]
pub enum EpisodicMemory {
  Table,
  Stability,
  Difficulty,
}
