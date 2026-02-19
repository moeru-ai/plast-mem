use plastmem_shared::AppError;

/// Target dimension for embeddings.
const TARGET_DIM: usize = 1024;
/// Threshold for determining if L2 normalization is needed.
const L2_NORM_TOLERANCE: f32 = 1e-6;

/// Process embedding vector to ensure it's L2 normalized with exactly 1024 dimensions.
///
/// - If dim > 1024: truncate to 1024 and L2 normalize
/// - If dim == 1024: check if already L2 normalized, normalize if not
/// - If dim < 1024: return error
pub fn process_embedding(mut vec: Vec<f32>) -> Result<Vec<f32>, AppError> {
  match vec.len() {
    d if d > TARGET_DIM => {
      // Truncate to 1024 and L2 normalize
      vec.truncate(TARGET_DIM);
      l2_normalize(&mut vec);
      Ok(vec)
    }
    d if d == TARGET_DIM => {
      // Check if already L2 normalized
      let norm_sq: f32 = vec.iter().map(|x| x * x).sum();
      if (norm_sq - 1.0).abs() > L2_NORM_TOLERANCE {
        l2_normalize(&mut vec);
      }
      Ok(vec)
    }
    d => Err(AppError::new(anyhow::anyhow!(
      "embedding dimension {} is less than required {}",
      d,
      TARGET_DIM
    ))),
  }
}

/// L2 normalize a vector in-place.
fn l2_normalize(vec: &mut [f32]) {
  let norm_sq: f32 = vec.iter().map(|x| x * x).sum();
  let norm = norm_sq.sqrt();
  if norm > 1e-12 {
    for x in vec.iter_mut() {
      *x /= norm;
    }
  }
}
