use sea_orm_migration::{prelude::*, sea_orm::Statement};

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
          .col(ColumnDef::new(ConversationMessage::Id).uuid().not_null().primary_key())
          .col(ColumnDef::new(ConversationMessage::ConversationId).uuid().not_null())
          .col(
            ColumnDef::new(ConversationMessage::Seq)
              .big_integer()
              .not_null(),
          )
          .col(ColumnDef::new(ConversationMessage::Role).string().not_null())
          .col(ColumnDef::new(ConversationMessage::Content).text().not_null())
          .col(
            ColumnDef::new(ConversationMessage::Timestamp)
              .timestamp_with_time_zone()
              .not_null(),
          )
          .col(
            ColumnDef::new(ConversationMessage::CreatedAt)
              .timestamp_with_time_zone()
              .not_null(),
          )
          .col(ColumnDef::new(ConversationMessage::Source).string().null())
          .col(ColumnDef::new(ConversationMessage::ImportId).uuid().null())
          .to_owned(),
      )
      .await?;

    manager
      .create_index(
        Index::create()
          .name("idx_conversation_message_conversation_seq_unique")
          .table(ConversationMessage::Table)
          .col(ConversationMessage::ConversationId)
          .col(ConversationMessage::Seq)
          .unique()
          .to_owned(),
      )
      .await?;

    manager
      .create_index(
        Index::create()
          .name("idx_conversation_message_conversation_seq")
          .table(ConversationMessage::Table)
          .col(ConversationMessage::ConversationId)
          .col(ConversationMessage::Seq)
          .to_owned(),
      )
      .await?;

    manager
      .create_table(
        Table::create()
          .table(SegmentationState::Table)
          .if_not_exists()
          .col(ColumnDef::new(SegmentationState::ConversationId).uuid().not_null().primary_key())
          .col(
            ColumnDef::new(SegmentationState::NextMessageSeq)
              .big_integer()
              .not_null()
              .default(0_i64),
          )
          .col(
            ColumnDef::new(SegmentationState::NextUnsegmentedSeq)
              .big_integer()
              .not_null()
              .default(0_i64),
          )
          .col(
            ColumnDef::new(SegmentationState::OpenTailStartSeq)
              .big_integer()
              .null(),
          )
          .col(ColumnDef::new(SegmentationState::LastSeenSeq).big_integer().null())
          .col(
            ColumnDef::new(SegmentationState::EofSeen)
              .boolean()
              .not_null()
              .default(false),
          )
          .col(
            ColumnDef::new(SegmentationState::InProgressUntilSeq)
              .big_integer()
              .null(),
          )
          .col(
            ColumnDef::new(SegmentationState::InProgressSince)
              .timestamp_with_time_zone()
              .null(),
          )
          .col(
            ColumnDef::new(SegmentationState::LastClosedBoundaryContext)
              .json_binary()
              .null(),
          )
          .col(
            ColumnDef::new(SegmentationState::StrategyVersion)
              .string()
              .not_null()
              .default("span_v2"),
          )
          .col(
            ColumnDef::new(SegmentationState::CreatedAt)
              .timestamp_with_time_zone()
              .not_null(),
          )
          .col(
            ColumnDef::new(SegmentationState::UpdatedAt)
              .timestamp_with_time_zone()
              .not_null(),
          )
          .to_owned(),
      )
      .await?;

    manager
      .create_table(
        Table::create()
          .table(EpisodeSpan::Table)
          .if_not_exists()
          .col(ColumnDef::new(EpisodeSpan::Id).uuid().not_null().primary_key())
          .col(ColumnDef::new(EpisodeSpan::ConversationId).uuid().not_null())
          .col(ColumnDef::new(EpisodeSpan::StartSeq).big_integer().not_null())
          .col(ColumnDef::new(EpisodeSpan::EndSeq).big_integer().not_null())
          .col(ColumnDef::new(EpisodeSpan::BoundaryReason).string().not_null())
          .col(ColumnDef::new(EpisodeSpan::SurpriseLevel).string().not_null())
          .col(ColumnDef::new(EpisodeSpan::Status).string().not_null())
          .col(
            ColumnDef::new(EpisodeSpan::CreatedAt)
              .timestamp_with_time_zone()
              .not_null(),
          )
          .foreign_key(
            ForeignKey::create()
              .name("fk_episode_span_conversation_state")
              .from(EpisodeSpan::Table, EpisodeSpan::ConversationId)
              .to(SegmentationState::Table, SegmentationState::ConversationId),
          )
          .to_owned(),
      )
      .await?;

    manager
      .create_index(
        Index::create()
          .name("idx_episode_span_conversation_start_seq")
          .table(EpisodeSpan::Table)
          .col(EpisodeSpan::ConversationId)
          .col(EpisodeSpan::StartSeq)
          .to_owned(),
      )
      .await?;

    manager
      .get_connection()
      .execute_raw(Statement::from_string(
        manager.get_database_backend(),
        "ALTER TABLE episodic_memory \
         ADD COLUMN IF NOT EXISTS source_span_id uuid NULL, \
         ADD COLUMN IF NOT EXISTS derivation_status text NOT NULL DEFAULT 'derived';",
      ))
      .await?;

    manager
      .create_index(
        Index::create()
          .name("idx_episodic_memory_source_span_unique")
          .table(Alias::new("episodic_memory"))
          .col(Alias::new("source_span_id"))
          .unique()
          .to_owned(),
      )
      .await?;

    manager
      .get_connection()
      .execute_raw(Statement::from_string(
        manager.get_database_backend(),
        "ALTER TABLE message_queue \
         DROP COLUMN IF EXISTS messages, \
         DROP COLUMN IF EXISTS in_progress_fence, \
         DROP COLUMN IF EXISTS in_progress_since, \
         DROP COLUMN IF EXISTS prev_episode_summary, \
         DROP COLUMN IF EXISTS prev_episode_content;",
      ))
      .await?;

    Ok(())
  }

  async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    manager
      .get_connection()
      .execute_raw(Statement::from_string(
        manager.get_database_backend(),
        "ALTER TABLE episodic_memory \
         DROP COLUMN IF EXISTS source_span_id, \
         DROP COLUMN IF EXISTS derivation_status;",
      ))
      .await?;

    manager
      .drop_table(Table::drop().table(EpisodeSpan::Table).to_owned())
      .await?;

    manager
      .drop_table(Table::drop().table(SegmentationState::Table).to_owned())
      .await?;

    manager
      .drop_table(Table::drop().table(ConversationMessage::Table).to_owned())
      .await?;

    Ok(())
  }
}

#[derive(Iden)]
enum ConversationMessage {
  Table,
  Id,
  ConversationId,
  Seq,
  Role,
  Content,
  Timestamp,
  CreatedAt,
  Source,
  ImportId,
}

#[derive(Iden)]
enum SegmentationState {
  Table,
  ConversationId,
  NextMessageSeq,
  NextUnsegmentedSeq,
  OpenTailStartSeq,
  LastSeenSeq,
  EofSeen,
  InProgressUntilSeq,
  InProgressSince,
  LastClosedBoundaryContext,
  StrategyVersion,
  CreatedAt,
  UpdatedAt,
}

#[derive(Iden)]
enum EpisodeSpan {
  Table,
  Id,
  ConversationId,
  StartSeq,
  EndSeq,
  BoundaryReason,
  SurpriseLevel,
  Status,
  CreatedAt,
}
