use std::env;
use std::sync::LazyLock;

fn required_env(key: &str) -> String {
  env::var(key).unwrap_or_else(|_| panic!("env {key} must be set"))
}

fn seed_env(key: &str) -> Option<i64> {
  env::var(key)
    .ok()
    .and_then(|value| value.trim().parse::<i64>().ok())
}

fn bool_env(key: &str, default: bool) -> bool {
  env::var(key).ok().map_or(default, |value| {
    match value.trim().to_ascii_lowercase().as_str() {
      "1" | "true" | "yes" | "on" => true,
      "0" | "false" | "no" | "off" => false,
      _ => default,
    }
  })
}

fn u64_env(key: &str, default: u64) -> u64 {
  env::var(key)
    .ok()
    .and_then(|value| value.trim().parse::<u64>().ok())
    .unwrap_or(default)
}

fn usize_env(key: &str, default: usize) -> usize {
  env::var(key)
    .ok()
    .and_then(|value| value.trim().parse::<usize>().ok())
    .unwrap_or(default)
}

pub struct AppEnv {
  pub database_url: String,
  pub openai_base_url: String,
  pub openai_api_key: String,
  pub openai_chat_model: String,
  pub openai_chat_seed: Option<i64>,
  pub openai_embedding_model: String,
  pub openai_request_timeout_seconds: u64,
  pub enable_fsrs_review: bool,
  pub predict_calibrate_concurrency: usize,
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
      openai_chat_seed: seed_env("OPENAI_CHAT_SEED"),
      openai_embedding_model: required_env("OPENAI_EMBEDDING_MODEL"),
      openai_request_timeout_seconds: u64_env("OPENAI_REQUEST_TIMEOUT_SECONDS", 60),
      enable_fsrs_review: bool_env("ENABLE_FSRS_REVIEW", true),
      predict_calibrate_concurrency: usize_env("PREDICT_CALIBRATE_CONCURRENCY", 4),
    }
  }
}

pub static APP_ENV: LazyLock<AppEnv> = LazyLock::new(AppEnv::new);
