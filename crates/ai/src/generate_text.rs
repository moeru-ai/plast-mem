use anyhow::anyhow;
use async_openai::{
  Client,
  config::OpenAIConfig,
  types::chat::{ChatCompletionRequestMessage, CreateChatCompletionRequestArgs, ReasoningEffort},
};
use plastmem_shared::{APP_ENV, AppError};

use crate::embed_shared::request_chat_completion_with_retry;

pub async fn generate_text(
  messages: Vec<ChatCompletionRequestMessage>,
) -> Result<String, AppError> {
  let config = OpenAIConfig::new()
    .with_api_key(&APP_ENV.openai_api_key)
    .with_api_base(&APP_ENV.openai_base_url);

  let client = Client::with_config(config);

  #[allow(deprecated)]
  let mut request_builder = CreateChatCompletionRequestArgs::default();
  request_builder
    .model(&APP_ENV.openai_chat_model)
    .messages(messages)
    .reasoning_effort(ReasoningEffort::None);

  if let Some(seed) = APP_ENV.openai_chat_seed {
    request_builder.seed(seed);
  }

  let request = request_builder.build()?;

  let chat = client.chat();

  request_chat_completion_with_retry(|| chat.create(request.clone()))
    .await
    .map(|r| r.choices.into_iter())?
    .filter_map(|c| c.message.content)
    .next_back()
    .ok_or_else(|| anyhow!("empty message content").into())
}
