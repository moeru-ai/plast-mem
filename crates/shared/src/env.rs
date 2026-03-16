use std::env;
use std::sync::LazyLock;

fn required_env(key: &str) -> String {
  env::var(key).unwrap_or_else(|_| panic!("env {key} must be set"))
}

fn bool_env(key: &str, default: bool) -> bool {
  env::var(key)
    .ok()
    .map(|value| match value.trim().to_ascii_lowercase().as_str() {
      "1" | "true" | "yes" | "on" => true,
      "0" | "false" | "no" | "off" => false,
      _ => default,
    })
    .unwrap_or(default)
}

pub struct AppEnv {
  pub database_url: String,
  pub openai_base_url: String,
  pub openai_api_key: String,
  pub openai_chat_model: String,
  pub openai_embedding_model: String,
  pub enable_fsrs_review: bool,
}

impl AppEnv {
  fn new() -> Self {
    dotenvy::dotenv().ok();

    Self {
      database_url: required_env("DATABASE_URL"),
      openai_base_url: required_env("OPENAI_BASE_URL")
        .trim_end_matches('/')
        .to_owned(),
      openai_api_key: required_env("OPENAI_API_KEY"),
      openai_chat_model: required_env("OPENAI_CHAT_MODEL"),
      openai_embedding_model: required_env("OPENAI_EMBEDDING_MODEL"),
      enable_fsrs_review: bool_env("ENABLE_FSRS_REVIEW", true),
    }
  }
}

pub static APP_ENV: LazyLock<AppEnv> = LazyLock::new(AppEnv::new);
