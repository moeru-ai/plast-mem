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
/// Recursively fix a JSON schema for OpenAI strict mode:
/// - additionalProperties: false on all objects
/// - required must include all property keys
fn fix_schema_for_strict(schema: &mut serde_json::Value) {
  let Some(obj) = schema.as_object_mut() else { return };

  // OpenAI strict mode (draft 7): $ref must be the only key — strip siblings
  if obj.contains_key("$ref") {
    obj.retain(|k, _| k == "$ref");
    return;
  }

  // Convert oneOf of const strings → enum (OpenAI strict mode forbids oneOf)
  if let Some(one_of) = obj.get("oneOf").and_then(|v| v.as_array()).cloned() {
    let consts: Option<Vec<serde_json::Value>> =
      one_of.iter().map(|v| v.get("const").cloned()).collect();
    if let Some(values) = consts {
      obj.clear();
      obj.insert("type".to_owned(), serde_json::Value::String("string".to_owned()));
      obj.insert("enum".to_owned(), serde_json::Value::Array(values));
      return;
    }
  }

  // Unwrap anyOf [T, null] → T (OpenAI strict mode forbids anyOf; Option<T> uses this pattern)
  if let Some(any_of) = obj.get("anyOf").and_then(|v| v.as_array()).cloned() {
    let non_null: Vec<&serde_json::Value> =
      any_of.iter().filter(|v| v.get("type").and_then(|t| t.as_str()) != Some("null")).collect();
    if non_null.len() == 1 {
      let inner = non_null[0].clone();
      obj.clear();
      obj.extend(inner.as_object().cloned().unwrap_or_default());
      fix_schema_for_strict(schema);
      return;
    }
  }

  if obj.contains_key("properties") {
    let keys: Vec<serde_json::Value> = obj["properties"]
      .as_object()
      .map(|p| p.keys().map(|k| serde_json::Value::String(k.clone())).collect())
      .unwrap_or_default();
    obj.insert("required".to_owned(), serde_json::Value::Array(keys));
    obj.insert("additionalProperties".to_owned(), serde_json::Value::Bool(false));

    // Recurse into property schemas
    if let Some(props) = obj.get_mut("properties").and_then(|p| p.as_object_mut()) {
      for v in props.values_mut() {
        fix_schema_for_strict(v);
      }
    }
  }

  // Recurse into array items
  if let Some(items) = obj.get_mut("items") {
    fix_schema_for_strict(items);
  }

  // Recurse into definitions (schemars 0.x uses "definitions")
  if let Some(defs) = obj.get_mut("definitions").and_then(|d| d.as_object_mut()) {
    for v in defs.values_mut() {
      fix_schema_for_strict(v);
    }
  }

  // Recurse into $defs (schemars 1.x uses "$defs")
  if let Some(defs) = obj.get_mut("$defs").and_then(|d| d.as_object_mut()) {
    for v in defs.values_mut() {
      fix_schema_for_strict(v);
    }
  }
}

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
  let mut schema = serde_json::to_value(&schema)?;
  // OpenAI strict mode requires additionalProperties: false and all properties in required
  fix_schema_for_strict(&mut schema);

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
