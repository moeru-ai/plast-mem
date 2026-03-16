use plastmem_shared::AppError;

/// Default embedding dimension used across the AI integration.
pub const EMBEDDING_DIM: usize = 1024;
/// Threshold for determining if L2 normalization is needed.
const L2_NORM_TOLERANCE: f32 = 1e-6;

/// Process embedding vector to ensure it's L2 normalized with exactly EMBEDDING_DIM dimensions.
///
/// - If dim > EMBEDDING_DIM: truncate to EMBEDDING_DIM and L2 normalize
/// - If dim == EMBEDDING_DIM: check if already L2 normalized, normalize if not
/// - If dim < EMBEDDING_DIM: return error
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
