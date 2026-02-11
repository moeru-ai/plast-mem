use async_openai::types::{
  ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
  ChatCompletionRequestUserMessage,
};
use plast_mem_shared::AppError;
use serde::{Deserialize, Serialize};

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

// pub async fn summarize_messages(messages: &[InputMessage]) -> Result<String, AppError> {
//   let system = ChatCompletionRequestSystemMessage::from(
//     "You are a professional summarizer. Provide a clear and concise summary",
//     // TODO: MAYBE:
//     // Provide a summary in bullet point format. (bullet-point)
//     // Provide a summary in paragraph format. (paragraph)
//     // Provide a very concise summary in 1-2 sentences. (concise)
//   );

//   let user = ChatCompletionRequestUserMessage::from(format_input_messages(messages));

//   generate_text(vec![
//     ChatCompletionRequestMessage::System(system),
//     ChatCompletionRequestMessage::User(user),
//   ])
//   .await
// }

/// Summarizes messages with optional significance check.
///
/// When `check` is true:
/// - Uses a specialized prompt to determine if the conversation contains significant content
/// - Returns `Ok(None)` if the LLM decides the content is trivial (replies with 'SKIP')
/// - Returns `Ok(Some(summary))` if the LLM decides to create a memory (replies with 'CREATE: <summary>')
///
/// When `check` is false:
/// - Directly generates and returns a summary
pub async fn summarize_messages_with_check(
  messages: &[InputMessage],
  check: bool,
) -> Result<Option<String>, AppError> {
  let formatted_messages = format_input_messages(messages);

  let system_prompt = if check {
    "You are an event segmentation analyzer. Analyze the conversation and decide if it contains significant content worth remembering as an episodic memory.\n\
     If the conversation is meaningful (contains important information, events, or context), reply with 'CREATE: ' followed by a concise summary.\n\
     If the conversation is trivial (greetings, small talk, or unimportant exchanges), reply with 'SKIP'.\n\
     Be selective - only mark as CREATE if there's substantive content."
  } else {
    "You are a professional summarizer. Provide a clear and concise summary of the following conversation."
  };

  let system = ChatCompletionRequestSystemMessage::from(system_prompt);
  let user = ChatCompletionRequestUserMessage::from(formatted_messages);

  // TODO: structured output
  let response = generate_text(vec![
    ChatCompletionRequestMessage::System(system),
    ChatCompletionRequestMessage::User(user),
  ])
  .await?;

  if check {
    let trimmed = response.trim();
    if trimmed.starts_with("CREATE:") {
      let summary = trimmed.strip_prefix("CREATE:").unwrap_or(trimmed).trim();
      if !summary.is_empty() {
        Ok(Some(summary.to_string()))
      } else {
        // If summary is empty after CREATE:, still create but use full response
        Ok(Some(trimmed.to_string()))
      }
    } else {
      // SKIP or any other response means don't create
      Ok(None)
    }
  } else {
    Ok(Some(response))
  }
}
