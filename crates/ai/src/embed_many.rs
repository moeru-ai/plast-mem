use anyhow::anyhow;
use async_openai::{Client, config::OpenAIConfig, types::embeddings::CreateEmbeddingRequestArgs};
use plastmem_shared::{APP_ENV, AppError};
use sea_orm::prelude::PgVector;

/// Embed multiple texts in a single API call.
///
/// Returns one `PgVector` per input, in the same order.
pub async fn embed_many(inputs: &[String]) -> Result<Vec<PgVector>, AppError> {
  if inputs.is_empty() {
    return Ok(vec![]);
  }

  let config = OpenAIConfig::new()
    .with_api_key(&APP_ENV.openai_api_key)
    .with_api_base(&APP_ENV.openai_base_url);

  let client = Client::with_config(config);

  let request = CreateEmbeddingRequestArgs::default()
    .model(&APP_ENV.openai_embedding_model)
    .input(inputs.to_vec())
    .dimensions(1024u32)
    .build()?;

  let response = client.embeddings().create(request).await?;

  // Sort by index to ensure ordering matches input
  let mut data = response.data;
  data.sort_by_key(|e| e.index);

  if data.len() != inputs.len() {
    return Err(
      anyhow!(
        "embedding count mismatch: expected {}, got {}",
        inputs.len(),
        data.len()
      )
      .into(),
    );
  }

  Ok(
    data
      .into_iter()
      .map(|e| PgVector::from(e.embedding))
      .collect(),
  )
}
