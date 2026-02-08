use async_openai::{
  Client,
  config::OpenAIConfig,
  types::{
    ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestUserMessageArgs,
    CreateChatCompletionRequestArgs, CreateEmbeddingRequestArgs,
  },
};
use sea_orm::prelude::PgVector;
use serde::{Deserialize, Serialize};

use plast_mem_shared::{APP_ENV, AppError};

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

// Lilia: Config LLM provider
fn openai_client() -> Client<OpenAIConfig> {
  let config = OpenAIConfig::new()
    .with_api_key(&APP_ENV.openai_api_key)
    .with_api_base(&APP_ENV.openai_base_url);

  Client::with_config(config)
}

fn chat_model() -> &'static str {
  &APP_ENV.openai_chat_model
}

fn embedding_model() -> &'static str {
  &APP_ENV.openai_embedding_model
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
  let client = openai_client();
  let model = chat_model();

  let system = ChatCompletionRequestSystemMessageArgs::default()
    .content("Provide a clear and concise summary")
    .build()?;

  let user = ChatCompletionRequestUserMessageArgs::default()
    .content(format_input_messages(messages))
    .build()?;

  let request = CreateChatCompletionRequestArgs::default()
    .model(model)
    .messages([system.into(), user.into()])
    .build()?;

  let response = client.chat().create(request).await?;
  let summary = response
    .choices
    .first()
    .and_then(|c| c.message.content.clone())
    .unwrap_or_default();

  Ok(summary)
}

pub async fn embed_text(text: &str) -> Result<PgVector, AppError> {
  let client = openai_client();
  let model = embedding_model();

  let request = CreateEmbeddingRequestArgs::default()
    .model(model)
    .input(text)
    .dimensions(1024u32)
    .build()?;

  let response = client.embeddings().create(request).await?;
  let embedding = response
    .data
    .first()
    .map(|item| item.embedding.clone())
    .unwrap_or_default();

  Ok(PgVector::from(embedding))
}

pub async fn decide_split(
  recent: &[InputMessage],
  incoming: &InputMessage,
) -> Result<bool, AppError> {
  let client = openai_client();
  let model = chat_model();

  let system = ChatCompletionRequestSystemMessageArgs::default()
    .content(
      "Decide whether the incoming message starts a new topic. Reply with 'split' or 'nosplit'.",
    )
    .build()?;

  let user = ChatCompletionRequestUserMessageArgs::default()
    .content(format!(
      "Recent:\n{}\n\nIncoming:\n{}",
      format_input_messages(recent),
      format_input_messages(std::slice::from_ref(incoming))
    ))
    .build()?;

  let request = CreateChatCompletionRequestArgs::default()
    .model(model)
    .messages([system.into(), user.into()])
    .build()?;

  let response = client.chat().create(request).await?;
  let content = response
    .choices
    .first()
    .and_then(|c| c.message.content.clone())
    .unwrap_or_default()
    .to_lowercase();

  Ok(content.contains("split"))
}
