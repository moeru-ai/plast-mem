use chrono::{DateTime, Utc};
use plastmem_ai::{
  ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
  ChatCompletionRequestUserMessage, embed_batch, generate_object,
};
use plastmem_entities::semantic_memory;
use plastmem_shared::{AppError, Message};
use schemars::JsonSchema;
use sea_orm::{
  ActiveModelTrait, ConnectionTrait, DatabaseConnection, DbBackend, FromQueryResult,
  IntoActiveModel, Statement, prelude::PgVector,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

// ──────────────────────────────────────────────────
// Domain model
// ──────────────────────────────────────────────────

#[derive(Debug, Serialize, Clone, ToSchema)]
pub struct SemanticFact {
  pub id: Uuid,
  pub subject: String,
  pub predicate: String,
  pub object: String,
  pub fact: String,
  pub source_ids: Vec<Uuid>,
  pub valid_at: DateTime<Utc>,
  pub invalid_at: Option<DateTime<Utc>>,
  #[serde(skip)]
  pub embedding: PgVector,
  pub created_at: DateTime<Utc>,
}

impl SemanticFact {
  pub fn from_model(model: semantic_memory::Model) -> Self {
    Self {
      id: model.id,
      subject: model.subject,
      predicate: model.predicate,
      object: model.object,
      fact: model.fact,
      source_ids: model.source_ids,
      valid_at: model.valid_at.with_timezone(&Utc),
      invalid_at: model.invalid_at.map(|dt| dt.with_timezone(&Utc)),
      embedding: model.embedding,
      created_at: model.created_at.with_timezone(&Utc),
    }
  }

  /// Check if this fact is a procedural / behavioral guideline.
  pub fn is_behavioral(&self) -> bool {
    self.subject == "assistant"
      && (self.predicate == "should"
        || self.predicate == "should_not"
        || self.predicate.starts_with("should_when_")
        || self.predicate.starts_with("responds_to_"))
  }

  /// Retrieve semantic facts using vector-only search.
  /// Only active facts (`invalid_at IS NULL`) are returned.
  pub async fn retrieve(
    query: &str,
    limit: u64,
    db: &DatabaseConnection,
  ) -> Result<Vec<(Self, f64)>, AppError> {
    let query_embedding = plastmem_ai::embed(query).await?;

    let sql = r"
    SELECT
      id, subject, predicate, object, fact, source_ids,
      valid_at, invalid_at, embedding, created_at,
      1 - (embedding <=> $1) AS score
    FROM semantic_memory
    WHERE invalid_at IS NULL
    ORDER BY embedding <=> $1
    LIMIT $2;
    ";

    let stmt = Statement::from_sql_and_values(
      DbBackend::Postgres,
      sql,
      vec![query_embedding.into(), (limit as i64).into()],
    );

    let rows = db.query_all_raw(stmt).await?;
    let mut results = Vec::with_capacity(rows.len());

    for row in rows {
      let model = semantic_memory::Model::from_query_result(&row, "")?;
      let score: f64 = row.try_get("", "score")?;
      let fact = Self::from_model(model);
      results.push((fact, score));
    }

    Ok(results)
  }
}

// ──────────────────────────────────────────────────
// LLM extraction types
// ──────────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SemanticExtractionOutput {
  pub facts: Vec<ExtractedFact>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExtractedFact {
  pub subject: String,
  pub predicate: String,
  pub object: String,
  /// Natural language sentence describing the fact
  pub fact: String,
}

// ──────────────────────────────────────────────────
// Extraction prompt
// ──────────────────────────────────────────────────

const EXTRACTION_SYSTEM_PROMPT: &str = "\
Extract lasting knowledge from this conversation segment.

Categories to extract:
1. Facts about the user (preferences, personal info, relationships)
2. Facts about the relationship (\"we\" subject)
3. Behavioral rules for the assistant:
   - Communication preferences the user has expressed
   - Topics to avoid or emphasize
   - Interaction patterns and rituals
   - Conditional behavior (when X happens, do Y)

Rules:
1. Only extract long-term facts. Ignore transient states (\"I'm hungry now\" is NOT a fact).
2. Use subject-predicate-object format.
3. Include a natural language `fact` sentence for each triple.
4. Preferences, habits, personal info, relationships, and significant events are good candidates.
5. For behavioral rules, use subject = \"assistant\".
6. If no lasting facts can be extracted, return an empty `facts` array.

Predicate taxonomy (use these when applicable; create new ones if needed):

  Personal: likes, dislikes, prefers, lives_in, works_at, age_is, name_is
  Knowledge: is_interested_in, has_experience_with, knows_about
  Relational: communicate_in_style, relationship_is, has_shared_reference, has_routine
  Behavioral: should, should_not, should_when_[context], responds_to_[trigger]_with";

// ──────────────────────────────────────────────────
// Extraction gating
// ──────────────────────────────────────────────────

/// Minimum content length to consider extraction (characters).
const GATE_MIN_CONTENT_LEN: usize = 50;
/// Surprise threshold below which short episodes are skipped.
const GATE_MAX_SURPRISE: f32 = 0.3;

/// Check if an episode should be skipped (too short AND unsurprising).
fn should_skip_extraction(content_len: usize, surprise: f32) -> bool {
  content_len < GATE_MIN_CONTENT_LEN && surprise < GATE_MAX_SURPRISE
}

// ──────────────────────────────────────────────────
// Deduplication threshold
// ──────────────────────────────────────────────────

/// Cosine similarity threshold for embedding-based deduplication.
/// Facts with similarity above this are considered true duplicates.
const DEDUPE_THRESHOLD: f64 = 0.95;

// ──────────────────────────────────────────────────
// Upsert (embedding-based dedupe)
// ──────────────────────────────────────────────────

/// Find existing active facts similar to the given embedding.
/// Returns all facts above the similarity threshold, ordered by similarity (highest first).
async fn find_similar_facts(
  embedding: &PgVector,
  threshold: f64,
  db: &DatabaseConnection,
) -> Result<Vec<semantic_memory::Model>, AppError> {
  let sql = r"
  SELECT
    id, subject, predicate, object, fact, source_ids,
    valid_at, invalid_at, embedding, created_at,
    1 - (embedding <=> $1) AS similarity
  FROM semantic_memory
  WHERE invalid_at IS NULL
    AND 1 - (embedding <=> $1) > $2
  ORDER BY similarity DESC
  LIMIT 5;
  ";

  let stmt = Statement::from_sql_and_values(
    DbBackend::Postgres,
    sql,
    vec![embedding.clone().into(), threshold.into()],
  );

  let rows = db.query_all_raw(stmt).await?;
  let mut results = Vec::with_capacity(rows.len());
  for row in rows {
    let model = semantic_memory::Model::from_query_result(&row, "")?;
    results.push(model);
  }
  Ok(results)
}

/// Append source IDs to an existing fact (merge duplicates).
/// Skips IDs that are already present to avoid duplicates.
async fn append_source_ids(
  fact_id: Uuid,
  existing_source_ids: &[Uuid],
  new_source_ids: &[Uuid],
  db: &DatabaseConnection,
) -> Result<(), AppError> {
  // Filter out IDs that already exist
  let existing_set: std::collections::HashSet<_> = existing_source_ids.iter().collect();
  let ids_to_add: Vec<_> = new_source_ids
    .iter()
    .filter(|id| !existing_set.contains(id))
    .copied()
    .collect();

  if ids_to_add.is_empty() {
    return Ok(());
  }

  // Build parameterized query: UNNEST($2::uuid[]) for safe array handling
  let sql = r#"
    UPDATE semantic_memory
    SET source_ids = source_ids || (SELECT ARRAY_AGG(x) FROM UNNEST($2::uuid[]) AS x)
    WHERE id = $1
  "#;

  let stmt = Statement::from_sql_and_values(
    DbBackend::Postgres,
    sql,
    vec![
      fact_id.into(),
      sea_orm::Value::Array(
        sea_orm::sea_query::ArrayType::Uuid,
        Some(Box::new(ids_to_add.into_iter().map(Into::into).collect())),
      ),
    ],
  );
  db.execute_raw(stmt).await?;
  Ok(())
}

/// Upsert a fact: deduplicate by embedding similarity, merge source_ids or insert new.
/// When multiple similar facts exist, merges into the most similar one.
async fn upsert_fact(
  extracted: &ExtractedFact,
  embedding: PgVector,
  source_episode_id: Uuid,
  db: &DatabaseConnection,
) -> Result<(), AppError> {
  // 1. Find highly similar existing facts (strict threshold)
  let similar = find_similar_facts(&embedding, DEDUPE_THRESHOLD, db).await?;

  // 2. If any similar facts found, merge source_ids into the most similar one
  // The results are already ordered by similarity DESC
  if let Some(existing) = similar.first() {
    tracing::debug!(
      existing_id = %existing.id,
      fact = %extracted.fact,
      similar_count = similar.len(),
      "Merging duplicate semantic fact"
    );
    append_source_ids(existing.id, &existing.source_ids, &[source_episode_id], db).await?;
    return Ok(());
  }

  // 3. No match → insert as new fact
  let id = Uuid::now_v7();
  let now = Utc::now();
  let model = semantic_memory::Model {
    id,
    subject: extracted.subject.clone(),
    predicate: extracted.predicate.clone(),
    object: extracted.object.clone(),
    fact: extracted.fact.clone(),
    source_ids: vec![source_episode_id],
    valid_at: now.into(),
    invalid_at: None,
    embedding,
    created_at: now.into(),
  };

  model.into_active_model().insert(db).await?;

  tracing::debug!(
    fact = %extracted.fact,
    "Inserted new semantic fact"
  );

  Ok(())
}

// ──────────────────────────────────────────────────
// End-to-end extraction pipeline
// ──────────────────────────────────────────────────

/// Process semantic extraction for an episode.
///
/// This is the full pipeline:
/// 1. Extraction gate (skip low-information episodes)
/// 2. Build prompt (surprise-aware)
/// 3. LLM extraction
/// 4. Batch embed all extracted facts
/// 5. Upsert each fact (embedding-based dedupe)
pub async fn process_extraction(
  episode_id: Uuid,
  summary: &str,
  messages: &[Message],
  surprise: f32,
  db: &DatabaseConnection,
) -> Result<(), AppError> {
  // 1. Extraction gate
  let content_len: usize = messages.iter().map(|m| m.content.len()).sum();
  if should_skip_extraction(content_len, surprise) {
    tracing::debug!(
      episode_id = %episode_id,
      content_len,
      surprise,
      "Skipping semantic extraction (gated)"
    );
    return Ok(());
  }

  // 2. Build prompt
  let mut system_prompt = EXTRACTION_SYSTEM_PROMPT.to_string();
  if surprise >= 0.85 {
    system_prompt.push_str(&format!(
      "\n\nThis episode had a surprise score of {surprise:.2}/1.0. \
       Extract facts more thoroughly — pay attention to novel or unexpected information."
    ));
  }

  let conversation = messages
    .iter()
    .map(std::string::ToString::to_string)
    .collect::<Vec<_>>()
    .join("\n");

  let user_content = format!("Episode summary: {summary}\n\nConversation:\n{conversation}");

  let system = ChatCompletionRequestSystemMessage::from(system_prompt.as_str());
  let user = ChatCompletionRequestUserMessage::from(user_content);

  // 3. LLM extraction
  let output = generate_object::<SemanticExtractionOutput>(
    vec![
      ChatCompletionRequestMessage::System(system),
      ChatCompletionRequestMessage::User(user),
    ],
    "semantic_extraction".to_owned(),
    Some("Extract lasting knowledge as semantic facts".to_owned()),
  )
  .await?;

  tracing::info!(
    episode_id = %episode_id,
    facts_count = output.facts.len(),
    "Semantic extraction completed"
  );

  if output.facts.is_empty() {
    return Ok(());
  }

  // 4. Batch embed all extracted facts
  let fact_texts: Vec<String> = output.facts.iter().map(|f| f.fact.clone()).collect();
  let embeddings = if fact_texts.is_empty() {
    vec![]
  } else {
    embed_batch(&fact_texts).await?
  };

  // 5. Upsert each fact
  for (extracted, embedding) in output.facts.iter().zip(embeddings.into_iter()) {
    upsert_fact(extracted, embedding, episode_id, db).await?;
  }

  Ok(())
}
