use std::env;
use std::sync::LazyLock;

fn required_env(key: &str) -> String {
  env::var(key).expect(&format!("env {key} must be set"))
}

pub struct AppEnv {
  pub database_url: String,
  pub openai_base_url: String,
  pub openai_api_key: String,
  pub openai_chat_model: String,
  pub openai_embedding_model: String,
}

impl AppEnv {
  fn new() -> Self {
    dotenvy::dotenv().ok();

    Self {
      database_url: required_env("DATABASE_URL"),
      openai_base_url: required_env("OPENAI_BASE_URL"),
      openai_api_key: required_env("OPENAI_API_KEY"),
      openai_chat_model: required_env("OPENAI_CHAT_MODEL"),
      openai_embedding_model: required_env("OPENAI_EMBEDDING_MODEL"),
    }
  }
}

pub static APP_ENV: LazyLock<AppEnv> = LazyLock::new(AppEnv::new);
