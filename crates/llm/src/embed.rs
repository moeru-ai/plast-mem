use anyhow::anyhow;
use async_openai::{Client, config::OpenAIConfig, types::CreateEmbeddingRequestArgs};
use plast_mem_shared::{APP_ENV, AppError};
use sea_orm::prelude::PgVector;

pub async fn embed(input: &str) -> Result<PgVector, AppError> {
  let config = OpenAIConfig::new()
    .with_api_key(&APP_ENV.openai_api_key)
    .with_api_base(&APP_ENV.openai_base_url);

  let client = Client::with_config(config);

  let request = CreateEmbeddingRequestArgs::default()
    .model(&APP_ENV.openai_embedding_model)
    .input(input)
    .dimensions(1024u32)
    .build()?;

  client
    .embeddings()
    .create(request)
    .await
    .map(|r| r.data.into_iter())?
    .map(|e| PgVector::from(e.embedding))
    .last()
    .ok_or(anyhow!("empty embedding").into())
}
