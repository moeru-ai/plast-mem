use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

mod data;
pub use data::{EventData, EventDataToString};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Event {
  pub id: Uuid,
  pub timestamp: DateTime<Utc>,
  #[serde(flatten)]
  pub data: EventData,
}

impl Event {
  pub fn new(data: EventData, timestamp: DateTime<Utc>, id: Option<Uuid>) -> Self {
    Self {
      id: id.unwrap_or(Uuid::now_v7()),
      timestamp,
      data,
    }
  }
}
