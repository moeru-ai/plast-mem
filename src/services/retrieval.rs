use std::collections::{HashMap, HashSet};

use crate::core::EpisodicMemory;

const HYBRID_WEIGHT_TFIDF: f32 = 0.5;
const HYBRID_WEIGHT_COSINE: f32 = 0.5;

// Lilia: This is a purely vibed in-memory retrieval logic...
// Database connection should be implemented ASAP to introduce ORM and pgvector etc...
pub fn retrieve_memories(
  query: &str,
  memories: &[EpisodicMemory],
  limit: usize,
) -> Vec<EpisodicMemory> {
  if memories.is_empty() {
    return vec![];
  }

  let query_tokens = tokenize(query);
  let doc_tokens: Vec<Vec<String>> = memories.iter().map(|m| tokenize(&m.search_text)).collect();

  let df_map = document_frequencies(&doc_tokens);

  let mut scored: Vec<(usize, f32, f32)> = memories
    .iter()
    .enumerate()
    .map(|(idx, mem)| {
      let tfidf = tfidf_score(&query_tokens, &doc_tokens[idx], &df_map, memories.len());
      let cosine = cosine_similarity(&query_tokens, &doc_tokens[idx]);
      let hybrid = HYBRID_WEIGHT_TFIDF * tfidf + HYBRID_WEIGHT_COSINE * cosine;
      (idx, hybrid, mem.fsrs.retrievability)
    })
    .collect();

  scored.sort_by(|a, b| b.1.total_cmp(&a.1));

  let mut top: Vec<(usize, f32, f32, usize)> = scored
    .into_iter()
    .take(limit)
    .enumerate()
    .map(|(rank, (idx, hybrid, retr))| (idx, hybrid, retr, rank))
    .collect();

  top.sort_by(|a, b| b.2.total_cmp(&a.2).then_with(|| a.3.cmp(&b.3)));

  top
    .into_iter()
    .map(|(idx, _, _, _)| memories[idx].clone())
    .collect()
}

fn tokenize(text: &str) -> Vec<String> {
  let mut normalized = String::with_capacity(text.len());
  for ch in text.chars() {
    if ch.is_alphanumeric() {
      normalized.push(ch.to_ascii_lowercase());
    } else {
      normalized.push(' ');
    }
  }
  normalized
    .split_whitespace()
    .filter(|s| !s.is_empty())
    .map(|s| s.to_string())
    .collect()
}

fn document_frequencies(docs: &[Vec<String>]) -> HashMap<String, usize> {
  let mut df = HashMap::new();
  for doc in docs {
    let unique: HashSet<&String> = doc.iter().collect();
    for term in unique {
      *df.entry(term.clone()).or_insert(0) += 1;
    }
  }
  df
}

fn tfidf_score(
  query_tokens: &[String],
  doc_tokens: &[String],
  df_map: &HashMap<String, usize>,
  total_docs: usize,
) -> f32 {
  if query_tokens.is_empty() || doc_tokens.is_empty() {
    return 0.0;
  }

  let mut counts = HashMap::new();
  for token in doc_tokens {
    *counts.entry(token).or_insert(0usize) += 1;
  }

  let doc_len = doc_tokens.len().max(1) as f32;
  let mut score = 0.0f32;

  for token in query_tokens {
    let tf = counts.get(token).copied().unwrap_or(0) as f32 / doc_len;
    if tf == 0.0 {
      continue;
    }
    let df = df_map.get(token).copied().unwrap_or(0) as f32;
    let idf = ((total_docs as f32 + 1.0) / (df + 1.0)).ln() + 1.0;
    score += tf * idf;
  }

  score
}

fn cosine_similarity(query_tokens: &[String], doc_tokens: &[String]) -> f32 {
  if query_tokens.is_empty() || doc_tokens.is_empty() {
    return 0.0;
  }

  let mut q_counts = HashMap::new();
  for token in query_tokens {
    *q_counts.entry(token).or_insert(0usize) += 1;
  }
  let mut d_counts = HashMap::new();
  for token in doc_tokens {
    *d_counts.entry(token).or_insert(0usize) += 1;
  }

  let mut dot = 0.0f32;
  let mut q_norm = 0.0f32;
  let mut d_norm = 0.0f32;

  for (token, qv) in &q_counts {
    let qv = *qv as f32;
    q_norm += qv * qv;
    if let Some(dv) = d_counts.get(token) {
      dot += qv * (*dv as f32);
    }
  }
  for dv in d_counts.values() {
    let dv = *dv as f32;
    d_norm += dv * dv;
  }

  if q_norm == 0.0 || d_norm == 0.0 {
    return 0.0;
  }

  dot / (q_norm.sqrt() * d_norm.sqrt())
}
