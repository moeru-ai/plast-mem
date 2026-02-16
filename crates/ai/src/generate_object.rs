use anyhow::anyhow;
use async_openai::{
  Client,
  config::OpenAIConfig,
  types::chat::{
    ChatCompletionRequestMessage, CreateChatCompletionRequestArgs, ResponseFormat,
    ResponseFormatJsonSchema,
  },
};
use plastmem_shared::{APP_ENV, AppError};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;

/// Generates a structured object
///
/// # Type Parameters
///
/// * `T` - The output type that implements `DeserializeOwned` and `JsonSchema`
///
/// # Arguments
///
/// * `messages` - The chat completion messages
/// * `schema_name` - A name for the schema
/// * `schema_description` - A description for the schema
///
/// # Example
///
/// ```rust
/// use schemars::JsonSchema;
/// use serde::Deserialize;
///
/// #[derive(Deserialize, JsonSchema)]
/// struct SurpriseScore {
///     score: f32,
///     reason: String,
/// }
///
/// let result = generate_object::<SurpriseScore>(
///     messages,
///     "surprise_score".to_owned(),
///     None,
/// ).await?;
/// ```
pub async fn generate_object<T>(
  messages: Vec<ChatCompletionRequestMessage>,
  schema_name: String,
  schema_description: Option<String>,
) -> Result<T, AppError>
where
  T: DeserializeOwned + JsonSchema,
{
  let config = OpenAIConfig::new()
    .with_api_key(&APP_ENV.openai_api_key)
    .with_api_base(&APP_ENV.openai_base_url);

  let client = Client::with_config(config);

  // Generate JSON schema from type
  let schema = schemars::schema_for!(T);
  let schema = serde_json::to_value(&schema)?;

  let request = CreateChatCompletionRequestArgs::default()
    .model(&APP_ENV.openai_chat_model)
    .messages(messages)
    .response_format(ResponseFormat::JsonSchema {
      json_schema: ResponseFormatJsonSchema {
        description: schema_description,
        name: schema_name,
        schema: Some(schema),
        strict: Some(true),
      },
    })
    .build()?;

  let response = client
    .chat()
    .create(request)
    .await
    .map(|r| r.choices.into_iter())?
    .find_map(|c| c.message.content)
    .ok_or_else(|| anyhow!("empty message content"))?;

  let result: T = serde_json::from_str(&response)?;

  Ok(result)
}
