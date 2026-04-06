pub use sea_orm_migration::*;

mod m20260216_01_create_message_queue_table;
mod m20260216_02_create_episodic_memory_table;
mod m20260218_01_create_semantic_memory_table;
mod m20260228_01_refactor_semantic_memory;
mod m20260305_01_refactor_episodic_memory_search;
mod m20260315_01_align_episodic_content;
mod m20260328_01_restore_semantic_fact_bm25;
mod m20260405_01_event_segmentation_v2;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
  fn migrations() -> Vec<Box<dyn MigrationTrait>> {
    vec![
      Box::new(m20260216_01_create_message_queue_table::Migration),
      Box::new(m20260216_02_create_episodic_memory_table::Migration),
      Box::new(m20260218_01_create_semantic_memory_table::Migration),
      Box::new(m20260228_01_refactor_semantic_memory::Migration),
      Box::new(m20260305_01_refactor_episodic_memory_search::Migration),
      Box::new(m20260315_01_align_episodic_content::Migration),
      Box::new(m20260328_01_restore_semantic_fact_bm25::Migration),
      Box::new(m20260405_01_event_segmentation_v2::Migration),
    ]
  }
}
