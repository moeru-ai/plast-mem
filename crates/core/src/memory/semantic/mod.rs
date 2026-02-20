mod consolidation;
pub use consolidation::{
  CONSOLIDATION_EPISODE_THRESHOLD, FLASHBULB_SURPRISE_THRESHOLD, process_consolidation,
};

use chrono::{DateTime, Utc};
use plastmem_ai::embed;
use plastmem_entities::semantic_memory;
use plastmem_shared::AppError;
use sea_orm::{
  ConnectionTrait, DatabaseConnection, DbBackend, FromQueryResult, Statement,
  prelude::PgVector,
};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

/// Number of candidates fetched per search leg (BM25 and vector) before RRF merging.
const RETRIEVAL_CANDIDATE_LIMIT: i64 = 100;

// ──────────────────────────────────────────────────
// Domain model
// ──────────────────────────────────────────────────

#[derive(Debug, Serialize, Clone, ToSchema)]
pub struct SemanticMemory {
  pub id: Uuid,
  pub conversation_id: Uuid,
  pub subject: String,
  pub predicate: String,
  pub object: String,
  pub fact: String,
  pub source_episodic_ids: Vec<Uuid>,
  pub valid_at: DateTime<Utc>,
  pub invalid_at: Option<DateTime<Utc>>,
  #[serde(skip)]
  pub embedding: PgVector,
  #[serde(skip)]
  pub created_at: DateTime<Utc>,
}

impl SemanticMemory {
  #[must_use]
  pub fn from_model(model: semantic_memory::Model) -> Self {
    Self {
      id: model.id,
      conversation_id: model.conversation_id,
      subject: model.subject,
      predicate: model.predicate,
      object: model.object,
      fact: model.fact,
      source_episodic_ids: model.source_episodic_ids,
      valid_at: model.valid_at.with_timezone(&Utc),
      invalid_at: model.invalid_at.map(|dt| dt.with_timezone(&Utc)),
      embedding: model.embedding,
      created_at: model.created_at.with_timezone(&Utc),
    }
  }

  /// Check if this fact is a procedural / behavioral guideline.
  #[must_use]
  pub fn is_behavioral(&self) -> bool {
    self.subject == "assistant"
      && (self.predicate == "should"
        || self.predicate == "should_not"
        || self.predicate.starts_with("should_when_")
        || self.predicate.starts_with("responds_to_"))
  }

  /// Retrieve semantic facts using hybrid BM25 + vector search with RRF.
  /// Only active facts (`invalid_at IS NULL`) from the specified conversation are returned.
  pub async fn retrieve(
    query: &str,
    limit: i64,
    conversation_id: Uuid,
    db: &DatabaseConnection,
  ) -> Result<Vec<(Self, f64)>, AppError> {
    let query_embedding = embed(query).await?;
    Self::retrieve_by_vector(query, query_embedding, limit, conversation_id, db).await
  }

  /// Like `retrieve`, but accepts a pre-computed embedding to avoid redundant API calls.
  pub(crate) async fn retrieve_by_vector(
    query: &str,
    query_embedding: PgVector,
    limit: i64,
    conversation_id: Uuid,
    db: &DatabaseConnection,
  ) -> Result<Vec<(Self, f64)>, AppError> {
    let sql = r"
    WITH
    fulltext AS (
      SELECT id, ROW_NUMBER() OVER (ORDER BY pdb.score(id) DESC) AS r
      FROM semantic_memory
      WHERE fact ||| $1 AND conversation_id = $2 AND invalid_at IS NULL
      LIMIT $3
    ),
    semantic AS (
      SELECT id, ROW_NUMBER() OVER (ORDER BY embedding <#> $4) AS r
      FROM semantic_memory
      WHERE conversation_id = $2 AND invalid_at IS NULL
      LIMIT $3
    ),
    rrf AS (
      SELECT id, 1.0 / (60 + r) AS s FROM fulltext
      UNION ALL
      SELECT id, 1.0 / (60 + r) AS s FROM semantic
    ),
    rrf_score AS (
      SELECT id, SUM(s) AS score
      FROM rrf
      GROUP BY id
    )
    SELECT
      m.id, m.conversation_id, m.subject, m.predicate, m.object, m.fact, m.source_episodic_ids,
      m.valid_at, m.invalid_at, m.embedding, m.created_at,
      r.score AS score
    FROM rrf_score r
    JOIN semantic_memory m USING (id)
    ORDER BY r.score DESC
    LIMIT $5;
    ";

    let stmt = Statement::from_sql_and_values(
      DbBackend::Postgres,
      sql,
      vec![
        query.to_owned().into(),      // $1
        conversation_id.into(),       // $2
        RETRIEVAL_CANDIDATE_LIMIT.into(), // $3: candidate limit
        query_embedding.into(),       // $4
        limit.into(),                 // $5
      ],
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
