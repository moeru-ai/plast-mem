use std::{future::Future, time::Duration};

use async_openai::error::OpenAIError;
use plastmem_shared::{APP_ENV, AppError};
use tokio::time::{sleep, timeout};
use tracing::error;

/// Default embedding dimension used across the AI integration.
pub const EMBEDDING_DIM: usize = 1024;
/// Threshold for determining if L2 normalization is needed.
const L2_NORM_TOLERANCE: f32 = 1e-6;
const EMBEDDING_MAX_ATTEMPTS: usize = 4;

const EMBEDDING_RETRY_DELAYS: [Duration; EMBEDDING_MAX_ATTEMPTS - 1] = [
  Duration::from_millis(500),
  Duration::from_secs(1),
  Duration::from_secs(2),
];

/// Process embedding vector to ensure it's L2 normalized with exactly `EMBEDDING_DIM` dimensions.
///
/// - If dim > `EMBEDDING_DIM`: truncate to `EMBEDDING_DIM` and L2 normalize
/// - If dim == `EMBEDDING_DIM`: check if already L2 normalized, normalize if not
/// - If dim < `EMBEDDING_DIM`: return error
pub fn process_embedding(mut vec: Vec<f32>) -> Result<Vec<f32>, AppError> {
  match vec.len() {
    d if d > EMBEDDING_DIM => {
      // Truncate to the configured dimension and L2 normalize
      vec.truncate(EMBEDDING_DIM);
      l2_normalize(&mut vec, None);
      Ok(vec)
    }
    d if d == EMBEDDING_DIM => {
      // Check if already L2 normalized
      let norm_sq: f32 = vec.iter().map(|x| x * x).sum();
      if (norm_sq - 1.0).abs() > L2_NORM_TOLERANCE {
        l2_normalize(&mut vec, Some(norm_sq));
      }
      Ok(vec)
    }
    d => Err(AppError::new(anyhow::anyhow!(
      "embedding dimension {d} is less than required {EMBEDDING_DIM}"
    ))),
  }
}

pub async fn request_embedding_with_retry<T, F, Fut>(mut operation: F) -> Result<T, AppError>
where
  F: FnMut() -> Fut,
  Fut: Future<Output = Result<T, OpenAIError>>,
{
  request_openai_with_retry(&mut operation, "Embedding request failed after retries").await
}

pub async fn request_chat_completion_with_retry<T, F, Fut>(
  mut operation: F,
) -> Result<T, AppError>
where
  F: FnMut() -> Fut,
  Fut: Future<Output = Result<T, OpenAIError>>,
{
  request_openai_with_retry(
    &mut operation,
    "Chat completion request failed after retries",
  )
  .await
}

async fn request_openai_with_retry<T, F, Fut>(
  operation: &mut F,
  final_error_message: &'static str,
) -> Result<T, AppError>
where
  F: FnMut() -> Fut,
  Fut: Future<Output = Result<T, OpenAIError>>,
{
  let timeout_duration = Duration::from_secs(APP_ENV.openai_request_timeout_seconds);
  let mut attempt = 0usize;

  loop {
    let result = timeout(timeout_duration, operation()).await.map_or_else(
      |_| Err(OpenAIRequestError::Timeout(timeout_duration)),
      |result| result.map_err(OpenAIRequestError::OpenAI),
    );

    match result {
      Ok(value) => return Ok(value),
      Err(err) => {
        if attempt >= EMBEDDING_MAX_ATTEMPTS - 1 || !is_retryable_request_error(&err) {
          error!(error = %err, "{final_error_message}");
          return Err(match err {
            OpenAIRequestError::OpenAI(err) => AppError::from(err),
            OpenAIRequestError::Timeout(duration) => AppError::new(anyhow::anyhow!(
              "OpenAI-compatible request timed out after {}s",
              duration.as_secs()
            )),
          });
        }

        sleep(EMBEDDING_RETRY_DELAYS[attempt]).await;
        attempt += 1;
      }
    }
  }
}

#[cfg(test)]
async fn retry_with_backoff<T, E, F, Fut, P, S, SFut>(
  operation: &mut F,
  mut should_retry: P,
  sleep_fn: &mut S,
) -> Result<T, E>
where
  F: FnMut() -> Fut,
  Fut: Future<Output = Result<T, E>>,
  P: FnMut(&E) -> bool,
  S: FnMut(Duration) -> SFut,
  SFut: Future<Output = ()>,
{
  let mut attempt = 0usize;

  loop {
    match operation().await {
      Ok(value) => return Ok(value),
      Err(err) => {
        if attempt >= EMBEDDING_MAX_ATTEMPTS - 1 || !should_retry(&err) {
          return Err(err);
        }

        sleep_fn(EMBEDDING_RETRY_DELAYS[attempt]).await;
        attempt += 1;
      }
    }
  }
}

fn is_retryable_openai_error(err: &OpenAIError) -> bool {
  match err {
    OpenAIError::Reqwest(reqwest_err) => {
      reqwest_err.is_timeout()
        || reqwest_err.is_connect()
        || matches!(
          reqwest_err.status().map(|status| status.as_u16()),
          Some(429 | 500..=599)
        )
        || has_retryable_message(&reqwest_err.to_string())
    }
    OpenAIError::ApiError(api_err) => {
      matches!(
        api_err.code.as_deref(),
        Some("rate_limit_exceeded" | "server_error" | "internal_error" | "server_overloaded")
      ) || has_retryable_message(&err.to_string())
    }
    _ => has_retryable_message(&err.to_string()),
  }
}

#[derive(Debug)]
enum OpenAIRequestError {
  OpenAI(OpenAIError),
  Timeout(Duration),
}

impl std::fmt::Display for OpenAIRequestError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::OpenAI(err) => write!(f, "{err}"),
      Self::Timeout(duration) => write!(
        f,
        "request timed out after {}s",
        duration.as_secs()
      ),
    }
  }
}

fn is_retryable_request_error(err: &OpenAIRequestError) -> bool {
  match err {
    OpenAIRequestError::OpenAI(err) => is_retryable_openai_error(err),
    OpenAIRequestError::Timeout(_) => true,
  }
}

fn has_retryable_message(message: &str) -> bool {
  let message = message.to_ascii_lowercase();
  message.contains("timeout")
    || message.contains("timed out")
    || message.contains("connection reset")
    || message.contains("connection refused")
    || message.contains("429")
    || message.contains("500")
    || message.contains("502")
    || message.contains("503")
    || message.contains("504")
}

/// L2 normalize a vector in-place.
fn l2_normalize(vec: &mut [f32], norm_sq: Option<f32>) {
  let norm_sq = norm_sq.unwrap_or_else(|| vec.iter().map(|x| x * x).sum());
  let norm = norm_sq.sqrt();
  if norm > 1e-12 {
    for x in vec.iter_mut() {
      *x /= norm;
    }
  }
}

#[cfg(test)]
mod tests {
  use std::{
    future::Future,
    pin::Pin,
    sync::{
      Arc,
      atomic::{AtomicUsize, Ordering},
    },
  };

  use super::retry_with_backoff;

  #[derive(Clone, Copy, Debug, Eq, PartialEq)]
  enum FakeError {
    Fatal,
    Retryable,
  }

  type BoxFutureResult<T> = Pin<Box<dyn Future<Output = Result<T, FakeError>> + Send>>;

  #[tokio::test]
  async fn retries_once_before_success() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_for_op = Arc::clone(&attempts);

    let mut operation = move || -> BoxFutureResult<u32> {
      let attempt = attempts_for_op.fetch_add(1, Ordering::SeqCst);
      Box::pin(async move {
        if attempt == 0 {
          Err(FakeError::Retryable)
        } else {
          Ok(7)
        }
      })
    };

    let result = retry_with_backoff(
      &mut operation,
      |err| *err == FakeError::Retryable,
      &mut |_| Box::pin(async {}) as Pin<Box<dyn Future<Output = ()>>>,
    )
    .await;

    assert_eq!(result, Ok(7));
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
  }

  #[tokio::test]
  async fn retries_up_to_fourth_attempt() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_for_op = Arc::clone(&attempts);

    let mut operation = move || -> BoxFutureResult<&'static str> {
      let attempt = attempts_for_op.fetch_add(1, Ordering::SeqCst);
      Box::pin(async move {
        if attempt < 3 {
          Err(FakeError::Retryable)
        } else {
          Ok("ok")
        }
      })
    };

    let result = retry_with_backoff(
      &mut operation,
      |err| *err == FakeError::Retryable,
      &mut |_| Box::pin(async {}) as Pin<Box<dyn Future<Output = ()>>>,
    )
    .await;

    assert_eq!(result, Ok("ok"));
    assert_eq!(attempts.load(Ordering::SeqCst), 4);
  }

  #[tokio::test]
  async fn returns_last_error_after_max_attempts() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_for_op = Arc::clone(&attempts);

    let mut operation = move || -> BoxFutureResult<()> {
      attempts_for_op.fetch_add(1, Ordering::SeqCst);
      Box::pin(async { Err(FakeError::Retryable) })
    };

    let result = retry_with_backoff(
      &mut operation,
      |err| *err == FakeError::Retryable,
      &mut |_| Box::pin(async {}) as Pin<Box<dyn Future<Output = ()>>>,
    )
    .await;

    assert_eq!(result, Err(FakeError::Retryable));
    assert_eq!(attempts.load(Ordering::SeqCst), 4);
  }

  #[tokio::test]
  async fn does_not_retry_non_retryable_error() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_for_op = Arc::clone(&attempts);

    let mut operation = move || -> BoxFutureResult<()> {
      attempts_for_op.fetch_add(1, Ordering::SeqCst);
      Box::pin(async { Err(FakeError::Fatal) })
    };

    let result = retry_with_backoff(
      &mut operation,
      |err| *err == FakeError::Retryable,
      &mut |_| Box::pin(async {}) as Pin<Box<dyn Future<Output = ()>>>,
    )
    .await;

    assert_eq!(result, Err(FakeError::Fatal));
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
  }
}
