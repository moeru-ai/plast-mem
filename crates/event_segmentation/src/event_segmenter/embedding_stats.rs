use plastmem_ai::cosine_similarity;

pub(super) fn build_prefix(embeddings: &[Vec<f32>]) -> Vec<Vec<f32>> {
  let dims = embeddings.first().map_or(0, Vec::len);
  let mut prefix = vec![vec![0.0; dims]];
  for embedding in embeddings {
    let mut next = prefix.last().cloned().unwrap_or_else(|| vec![0.0; dims]);
    for (value, current) in next.iter_mut().zip(embedding) {
      *value += current;
    }
    prefix.push(next);
  }
  prefix
}

pub(super) fn mean_vector(prefix: &[Vec<f32>], start: usize, end: usize) -> Vec<f32> {
  if start >= end || prefix.is_empty() {
    return Vec::new();
  }

  let count = (end - start) as f32;
  let mut mean = prefix[end]
    .iter()
    .zip(&prefix[start])
    .map(|(right, left)| (right - left) / count)
    .collect::<Vec<_>>();
  normalize(&mut mean);
  mean
}

pub(super) fn normalize(vector: &mut [f32]) {
  let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
  if norm > f32::EPSILON {
    for value in vector {
      *value /= norm;
    }
  }
}

pub(super) fn segment_cohesion(
  embeddings: &[Vec<f32>],
  prefix: &[Vec<f32>],
  start: usize,
  end: usize,
) -> f32 {
  if end <= start + 1 {
    return 1.0;
  }

  let mean = mean_vector(prefix, start, end);
  let total = embeddings[start..end]
    .iter()
    .map(|embedding| cosine_similarity(embedding, &mean))
    .sum::<f32>();
  (total / (end - start) as f32).clamp(-1.0, 1.0)
}
