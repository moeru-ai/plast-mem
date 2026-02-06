use std::sync::Arc;

use tokio::sync::RwLock;

use crate::core::{EpisodicMemory, MessageQueue};

#[derive(Clone, Debug, Default)]
pub struct AppState {
  pub message_queues: Arc<RwLock<Vec<MessageQueue>>>,
  pub memories: Arc<RwLock<Vec<EpisodicMemory>>>,
}

impl AppState {
  pub fn new() -> Self {
    Self::default()
  }
}
