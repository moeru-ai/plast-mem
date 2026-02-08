use std::env;
use std::sync::LazyLock;

pub struct AppEnv {
  pub database_url: String,
  pub openai_base_url: String,
  pub openai_api_key: String,
  pub openai_chat_model: String,
  pub openai_embedding_model: String,
}

impl AppEnv {
  fn new() -> Self {
    Self {
      database_url: env::var("DATABASE_URL").expect("DATABASE_URL must be set"),
      openai_base_url: env::var("OPENAI_BASE_URL").expect("OPENAI_BASE_URL must be set"),
      openai_api_key: env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY must be set"),
      openai_chat_model: env::var("OPENAI_CHAT_MODEL").expect("OPENAI_CHAT_MODEL must be set"),
      openai_embedding_model: env::var("OPENAI_EMBEDDING_MODEL")
        .expect("OPENAI_EMBEDDING_MODEL must be set"),
    }
  }
}

pub static APP_ENV: LazyLock<AppEnv> = LazyLock::new(AppEnv::new);
