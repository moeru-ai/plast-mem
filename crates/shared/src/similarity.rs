/// Compute cosine similarity between two embedding vectors.
///
/// Returns a value in [-1.0, 1.0] where 1.0 means identical direction.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
  if a.is_empty() || b.is_empty() {
    return 0.0;
  }
  debug_assert_eq!(a.len(), b.len(), "embedding dimensions must match");

  let mut dot = 0.0_f32;
  let mut norm_a = 0.0_f32;
  let mut norm_b = 0.0_f32;

  for (x, y) in a.iter().zip(b.iter()) {
    dot += x * y;
    norm_a += x * x;
    norm_b += y * y;
  }

  let denom = norm_a.sqrt() * norm_b.sqrt();
  if denom < 1e-6 {
    return 0.0;
  }
  dot / denom
}
