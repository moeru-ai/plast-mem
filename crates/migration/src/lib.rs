pub use sea_orm_migration::*;

mod m20260206_01_create_message_queue_table;
mod m20260206_02_create_episodic_memory_table;
mod m20260211_01_create_episodic_memory_bm25_index;
mod m20260213_01_add_pending_reviews_to_message_queue;
mod m20260215_01_add_event_model_to_message_queue;
mod m20260215_02_add_title_to_episodic_memory;
mod m20260215_03_add_event_model_embedding_to_message_queue;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
  fn migrations() -> Vec<Box<dyn MigrationTrait>> {
    vec![
      Box::new(m20260206_01_create_message_queue_table::Migration),
      Box::new(m20260206_02_create_episodic_memory_table::Migration),
      Box::new(m20260211_01_create_episodic_memory_bm25_index::Migration),
      Box::new(m20260213_01_add_pending_reviews_to_message_queue::Migration),
      Box::new(m20260215_01_add_event_model_to_message_queue::Migration),
      Box::new(m20260215_02_add_title_to_episodic_memory::Migration),
      Box::new(m20260215_03_add_event_model_embedding_to_message_queue::Migration),
    ]
  }
}
