use anyhow::anyhow;
use async_openai::{Client, config::OpenAIConfig, types::embeddings::CreateEmbeddingRequestArgs};
use plastmem_shared::{APP_ENV, AppError};
use sea_orm::prelude::PgVector;

use crate::embed_shared::{EMBEDDING_DIM, process_embedding, request_embedding_with_retry};

pub async fn embed(input: &str) -> Result<PgVector, AppError> {
  let embedding_dim = u32::try_from(EMBEDDING_DIM)
    .map_err(|_| anyhow!("EMBEDDING_DIM must fit in u32"))?;
  let config = OpenAIConfig::new()
    .with_api_key(&APP_ENV.openai_api_key)
    .with_api_base(&APP_ENV.openai_base_url);

  let client = Client::with_config(config);

  let request = CreateEmbeddingRequestArgs::default()
    .model(&APP_ENV.openai_embedding_model)
    .input(input)
    .dimensions(embedding_dim)
    .build()?;
  let embeddings = client.embeddings();

  let embedding = request_embedding_with_retry(|| embeddings.create(request.clone()))
    .await
    .map(|r| r.data.into_iter())?
    .map(|e| e.embedding)
    .next_back()
    .ok_or_else(|| anyhow!("empty embedding"))?;

  let processed = process_embedding(embedding)?;
  Ok(PgVector::from(processed))
}
