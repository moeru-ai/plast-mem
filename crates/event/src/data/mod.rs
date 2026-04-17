use chrono::{DateTime, Utc};
use enum_dispatch::enum_dispatch;
use serde::{Deserialize, Serialize};

mod message;
use message::MessageEventData;

#[enum_dispatch]
pub trait EventDataToString {
  fn to_string_with_timestamp(&self, timestamp: DateTime<Utc>) -> String;
  fn to_string_without_timestamp(&self) -> String;
}

#[enum_dispatch(EventDataToString)]
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum EventData {
  #[serde(rename = "message")]
  Message(MessageEventData),
  // #[serde(untagged)]
  // FallbackMessage(MessageEventData),
}
