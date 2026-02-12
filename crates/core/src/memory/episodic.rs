use crate::Message;
use chrono::{DateTime, Utc};
use fsrs::{FSRS, MemoryState};
use plast_mem_db_schema::episodic_memory;
use plast_mem_llm::embed;
use plast_mem_shared::AppError;
use sea_orm::{
  ConnectionTrait, DatabaseConnection, DbBackend, FromQueryResult, Statement, prelude::PgVector,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Default FSRS decay for retrievability calculation.
/// Uses FSRS-5 default: 0.0 means the crate applies its internal default.
const FSRS_DECAY: f32 = 0.0;
/// Candidate pool size for FSRS re-ranking.
const FSRS_CANDIDATE_LIMIT: u64 = 100;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EpisodicMemory {
  pub id: Uuid,
  pub conversation_id: Uuid,
  pub messages: Vec<Message>,
  pub content: String,
  pub embedding: PgVector,
  pub stability: f32,
  pub difficulty: f32,
  pub start_at: DateTime<Utc>,
  pub end_at: DateTime<Utc>,
  pub created_at: DateTime<Utc>,
  pub last_reviewed_at: DateTime<Utc>,
}

impl EpisodicMemory {
  pub fn from_model(model: episodic_memory::Model) -> Result<Self, AppError> {
    Ok(Self {
      id: model.id,
      conversation_id: model.conversation_id,
      messages: serde_json::from_value(model.messages)?,
      content: model.content,
      embedding: model.embedding,
      stability: model.stability,
      difficulty: model.difficulty,
      start_at: model.start_at.with_timezone(&Utc),
      end_at: model.end_at.with_timezone(&Utc),
      created_at: model.created_at.with_timezone(&Utc),
      last_reviewed_at: model.last_reviewed_at.with_timezone(&Utc),
    })
  }

  pub fn to_model(&self) -> Result<episodic_memory::Model, AppError> {
    Ok(episodic_memory::Model {
      id: self.id,
      conversation_id: self.conversation_id,
      messages: serde_json::to_value(self.messages.clone())?,
      content: self.content.clone(),
      embedding: self.embedding.clone(),
      stability: self.stability,
      difficulty: self.difficulty,
      start_at: self.start_at.into(),
      end_at: self.end_at.into(),
      created_at: self.created_at.into(),
      last_reviewed_at: self.last_reviewed_at.into(),
    })
  }

  pub async fn retrieve(
    query: &str,
    limit: u64,
    db: &DatabaseConnection,
  ) -> Result<Vec<(Self, f64)>, AppError> {
    let query_embedding = embed(query).await?;
    let fsrs = FSRS::new(None)?;

    // Fetch a fixed candidate pool for FSRS re-ranking
    let candidate_limit = FSRS_CANDIDATE_LIMIT;

    let retrieve_sql = r#"
    WITH
    fulltext AS (
      SELECT id, ROW_NUMBER() OVER (ORDER BY pdb.score(id) DESC) AS rank
      FROM episodic_memory
      WHERE content ||| $1
      LIMIT $2
    ),
    semantic AS (
      SELECT id, ROW_NUMBER() OVER (ORDER BY embedding <=> $3) AS rank
      FROM episodic_memory
      LIMIT $2
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
      m.id,
      m.conversation_id,
      m.messages,
      m.content,
      m.embedding,
      m.stability,
      m.difficulty,
      m.start_at,
      m.end_at,
      m.created_at,
      m.last_reviewed_at,
      r.score AS score
    FROM rrf_score r
    JOIN episodic_memory m USING (id)
    ORDER BY r.score DESC
    LIMIT $4;
    "#;

    let retrieve_stmt = Statement::from_sql_and_values(
      DbBackend::Postgres,
      retrieve_sql,
      vec![
        query.to_string().into(),
        candidate_limit.into(),
        query_embedding.clone().into(),
        candidate_limit.into(),
      ],
    );

    let rows = db.query_all_raw(retrieve_stmt).await?;
    let mut results = Vec::with_capacity(rows.len());
    let now = Utc::now();

    for row in rows {
      let model = episodic_memory::Model::from_query_result(&row, "")?;
      let rrf_score: f64 = row.try_get("", "score")?;
      let mem = EpisodicMemory::from_model(model)?;

      // FSRS re-ranking: multiply RRF score by retrievability
      let days_elapsed = (now - mem.last_reviewed_at).num_seconds().max(0) as u32 / 86400;
      let memory_state = MemoryState {
        stability: mem.stability,
        difficulty: mem.difficulty,
      };
      let retrievability = fsrs.current_retrievability(memory_state, days_elapsed, FSRS_DECAY);
      let final_score = rrf_score * retrievability as f64;

      results.push((mem, final_score));
    }

    // Re-sort by final score descending and truncate to requested limit
    results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    results.truncate(limit as usize);

    Ok(results)
  }
}
