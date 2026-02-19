/// Compute inner product between two embedding vectors.
///
/// Assumes both vectors are L2 normalized. Returns a value in [-1.0, 1.0]
/// where 1.0 means identical direction.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
  debug_assert_eq!(a.len(), b.len(), "embedding dimensions must match");
  if a.len() != b.len() || a.is_empty() {
    return 0.0;
  }

  let mut dot = 0.0_f64;

  for (&x, &y) in a.iter().zip(b.iter()) {
    dot += (x as f64) * (y as f64);
  }

  dot as f32
}
