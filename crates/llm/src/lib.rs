use async_openai::types::{
  ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
  ChatCompletionRequestUserMessage,
};
use serde::{Deserialize, Serialize};

use plast_mem_shared::AppError;

mod embed;
pub use embed::embed;

mod generate_text;
pub use generate_text::generate_text;

#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
pub enum Role {
  User,
  Assistant,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct InputMessage {
  pub role: Role,
  pub content: String,
}

fn format_input_messages(messages: &[InputMessage]) -> String {
  messages
    .iter()
    .map(|m| {
      let role = match m.role {
        Role::User => "user",
        Role::Assistant => "assistant",
      };
      format!("{}: {}", role, m.content)
    })
    .collect::<Vec<_>>()
    .join("\n")
}

pub async fn summarize_messages(messages: &[InputMessage]) -> Result<String, AppError> {
  let system = ChatCompletionRequestSystemMessage::from(
    "You are a professional summarizer. Provide a clear and concise summary",
    // TODO: MAYBE:
    // Provide a summary in bullet point format. (bullet-point)
    // Provide a summary in paragraph format. (paragraph)
    // Provide a very concise summary in 1-2 sentences. (concise)
  );

  let user = ChatCompletionRequestUserMessage::from(format_input_messages(messages));

  generate_text(vec![
    ChatCompletionRequestMessage::System(system),
    ChatCompletionRequestMessage::User(user),
  ])
  .await
}

pub async fn decide_split(
  recent: &[InputMessage],
  incoming: &InputMessage,
) -> Result<bool, AppError> {
  let system = ChatCompletionRequestSystemMessage::from(
    "Decide whether the incoming message starts a new topic. Reply with 'split' or 'nosplit'.",
  );

  let user = ChatCompletionRequestUserMessage::from(format!(
    "Recent:\n{}\n\nIncoming:\n{}",
    format_input_messages(recent),
    format_input_messages(std::slice::from_ref(incoming))
  ));

  let text = generate_text(vec![
    ChatCompletionRequestMessage::System(system),
    ChatCompletionRequestMessage::User(user),
  ])
  .await?;

  Ok(text.contains("split"))
}
