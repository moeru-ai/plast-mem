use chrono::{DateTime, Utc};
use plastmem_ai::embed;
use plastmem_entities::semantic_memory;
use plastmem_shared::AppError;
use sea_orm::{
  ConnectionTrait, DatabaseConnection, DbBackend, FromQueryResult, Statement, prelude::PgVector,
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
  pub category: String,
  pub fact: String,
  pub keywords: Vec<String>,
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
      category: model.category,
      fact: model.fact,
      keywords: model.keywords,
      source_episodic_ids: model.source_episodic_ids,
      valid_at: model.valid_at.with_timezone(&Utc),
      invalid_at: model.invalid_at.map(|dt| dt.with_timezone(&Utc)),
      embedding: model.embedding,
      created_at: model.created_at.with_timezone(&Utc),
    }
  }

  /// Check if this fact is a behavioral guideline for the assistant.
  #[must_use]
  pub fn is_behavioral(&self) -> bool {
    self.category == "guideline"
  }

  /// Retrieve semantic facts using hybrid BM25 + vector search with RRF.
  pub async fn retrieve(
    query: &str,
    limit: i64,
    conversation_id: Uuid,
    db: &DatabaseConnection,
    category: Option<&str>,
  ) -> Result<Vec<(Self, f64)>, AppError> {
    let query_embedding = embed(query).await?;
    Self::retrieve_by_embedding(query, query_embedding, limit, conversation_id, db, category).await
  }

  /// Like `retrieve`, but accepts a pre-computed embedding to avoid redundant API calls.
  pub async fn retrieve_by_embedding(
    query: &str,
    query_embedding: PgVector,
    limit: i64,
    conversation_id: Uuid,
    db: &DatabaseConnection,
    category: Option<&str>,
  ) -> Result<Vec<(Self, f64)>, AppError> {
    let sql = r"
    WITH
    fulltext AS (
      SELECT id, ROW_NUMBER() OVER (ORDER BY pdb.score(id) DESC) AS r
      FROM semantic_memory
      WHERE search_text ||| $1
        AND conversation_id = $2
        AND invalid_at IS NULL
        AND ($6::text IS NULL OR category = $6)
      LIMIT $3
    ),
    semantic AS (
      SELECT id, ROW_NUMBER() OVER (ORDER BY embedding <#> $4) AS r
      FROM semantic_memory
      WHERE conversation_id = $2
        AND invalid_at IS NULL
        AND ($6::text IS NULL OR category = $6)
      LIMIT $3
    ),
    rrf AS (
      SELECT id, 1.0 / (60 + r) AS s FROM fulltext
      UNION ALL
      SELECT id, 1.0 / (60 + r) AS s FROM semantic
    ),
    rrf_score AS (
      SELECT id, SUM(s)::float8 AS score
      FROM rrf
      GROUP BY id
    )
    SELECT
      m.id, m.conversation_id, m.category, m.fact, m.keywords, m.source_episodic_ids,
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
        query.to_owned().into(),
        conversation_id.into(),
        RETRIEVAL_CANDIDATE_LIMIT.into(),
        query_embedding.into(),
        limit.into(),
        category.map(|s| s.to_owned()).into(),
      ],
    );

    let rows = db.query_all_raw(stmt).await?;
    let mut results = Vec::with_capacity(rows.len());

    for row in rows {
      let model = semantic_memory::Model::from_query_result(&row, "")?;
      let score: f64 = row.try_get("", "score")?;
      results.push((Self::from_model(model), score));
    }

    Ok(results)
  }
}
