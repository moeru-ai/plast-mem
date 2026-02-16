/// Compute cosine similarity between two embedding vectors.
///
/// Returns a value in [-1.0, 1.0] where 1.0 means identical direction.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
  if a.is_empty() || b.is_empty() {
    return 0.0;
  }
  if a.len() != b.len() {
    return 0.0;
  }

  let mut dot = 0.0_f64;
  let mut norm_a = 0.0_f64;
  let mut norm_b = 0.0_f64;

  for (&x, &y) in a.iter().zip(b.iter()) {
    let x = x as f64;
    let y = y as f64;
    dot = x.mul_add(y, dot);
    norm_a = x.mul_add(x, norm_a);
    norm_b = y.mul_add(y, norm_b);
  }

  let denom = norm_a.sqrt() * norm_b.sqrt();
  if denom < 1e-12 {
    return 0.0;
  }

  (dot / denom) as f32
}
