pub use sea_orm_migration::*;

mod m20260417_01_create_conversation_message_table;
mod m20260417_02_create_segmentation_state_table;
mod m20260417_03_create_episode_span_table;
mod m20260417_04_create_pending_review_queue_table;
mod m20260417_05_create_episodic_memory_table;
mod m20260417_06_create_semantic_memory_table;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
  fn migrations() -> Vec<Box<dyn MigrationTrait>> {
    vec![
      Box::new(m20260417_01_create_conversation_message_table::Migration),
      Box::new(m20260417_02_create_segmentation_state_table::Migration),
      Box::new(m20260417_03_create_episode_span_table::Migration),
      Box::new(m20260417_04_create_pending_review_queue_table::Migration),
      Box::new(m20260417_05_create_episodic_memory_table::Migration),
      Box::new(m20260417_06_create_semantic_memory_table::Migration),
    ]
  }
}
