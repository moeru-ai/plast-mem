use anyhow::anyhow;
use async_openai::{
  Client,
  config::OpenAIConfig,
  types::chat::{ChatCompletionRequestMessage, CreateChatCompletionRequestArgs},
};
use plastmem_shared::{APP_ENV, AppError};

pub async fn generate_text(
  messages: Vec<ChatCompletionRequestMessage>,
) -> Result<String, AppError> {
  let config = OpenAIConfig::new()
    .with_api_key(&APP_ENV.openai_api_key)
    .with_api_base(&APP_ENV.openai_base_url);

  let client = Client::with_config(config);

  let request = CreateChatCompletionRequestArgs::default()
    .model(&APP_ENV.openai_chat_model)
    .messages(messages)
    .build()?;

  client
    .chat()
    .create(request)
    .await
    .map(|r| r.choices.into_iter())?
    .filter_map(|c| c.message.content)
    .next_back()
    .ok_or_else(|| anyhow!("empty message content").into())
}
