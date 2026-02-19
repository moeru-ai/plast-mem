use anyhow::anyhow;
use async_openai::{Client, config::OpenAIConfig, types::embeddings::CreateEmbeddingRequestArgs};
use plastmem_shared::{APP_ENV, AppError};
use sea_orm::prelude::PgVector;

use crate::embed_shared::process_embedding;

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

  let embedding = client
    .embeddings()
    .create(request)
    .await
    .map(|r| r.data.into_iter())?
    .map(|e| e.embedding)
    .next_back()
    .ok_or_else(|| anyhow!("empty embedding"))?;

  let processed = process_embedding(embedding)?;
  Ok(PgVector::from(processed))
}
