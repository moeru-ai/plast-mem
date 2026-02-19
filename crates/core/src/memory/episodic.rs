use chrono::{DateTime, Utc};
use fsrs::{DEFAULT_PARAMETERS, FSRS, FSRS6_DEFAULT_DECAY, MemoryState};
use plastmem_ai::embed;
use plastmem_entities::episodic_memory;
use plastmem_shared::{AppError, Message};

use sea_orm::{
  ColumnTrait, ConnectionTrait, DatabaseConnection, DbBackend, EntityTrait, FromQueryResult,
  QueryFilter, QueryOrder, Statement, prelude::{Expr, PgVector},
};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Serialize, Clone, ToSchema)]
pub struct EpisodicMemory {
  pub id: Uuid,
  pub conversation_id: Uuid,
  pub messages: Vec<Message>,
  pub title: String,
  pub summary: String,
  /// Vector embedding (internal use, not exposed in API)
  #[serde(skip)]
  pub embedding: PgVector,
  pub stability: f32,
  pub difficulty: f32,
  pub surprise: f32,
  pub start_at: DateTime<Utc>,
  pub end_at: DateTime<Utc>,
  pub created_at: DateTime<Utc>,
  pub last_reviewed_at: DateTime<Utc>,
  pub consolidated_at: Option<DateTime<Utc>>,
}

impl EpisodicMemory {
  pub fn from_model(model: episodic_memory::Model) -> Result<Self, AppError> {
    Ok(Self {
      id: model.id,
      conversation_id: model.conversation_id,
      messages: serde_json::from_value(model.messages)?,
      title: model.title,
      summary: model.summary,
      embedding: model.embedding,
      stability: model.stability,
      difficulty: model.difficulty,
      surprise: model.surprise,
      start_at: model.start_at.with_timezone(&Utc),
      end_at: model.end_at.with_timezone(&Utc),
      created_at: model.created_at.with_timezone(&Utc),
      last_reviewed_at: model.last_reviewed_at.with_timezone(&Utc),
      consolidated_at: model.consolidated_at.map(|dt| dt.with_timezone(&Utc)),
    })
  }

  pub fn to_model(&self) -> Result<episodic_memory::Model, AppError> {
    Ok(episodic_memory::Model {
      id: self.id,
      conversation_id: self.conversation_id,
      messages: serde_json::to_value(self.messages.clone())?,
      title: self.title.clone(),
      summary: self.summary.clone(),
      embedding: self.embedding.clone(),
      stability: self.stability,
      difficulty: self.difficulty,
      surprise: self.surprise,
      start_at: self.start_at.into(),
      end_at: self.end_at.into(),
      created_at: self.created_at.into(),
      last_reviewed_at: self.last_reviewed_at.into(),
      consolidated_at: self.consolidated_at.map(Into::into),
    })
  }

  /// Count unconsolidated episodes for a specific conversation.
  pub async fn count_unconsolidated_for_conversation(
    conversation_id: Uuid,
    db: &DatabaseConnection,
  ) -> Result<u64, AppError> {
    use sea_orm::PaginatorTrait;
    let count = episodic_memory::Entity::find()
      .filter(episodic_memory::Column::ConsolidatedAt.is_null())
      .filter(episodic_memory::Column::ConversationId.eq(conversation_id))
      .count(db)
      .await?;
    Ok(count)
  }

  /// Fetch unconsolidated episodes for a specific conversation.
  pub async fn fetch_unconsolidated_for_conversation(
    conversation_id: Uuid,
    db: &DatabaseConnection,
  ) -> Result<Vec<Self>, AppError> {
    let models = episodic_memory::Entity::find()
      .filter(episodic_memory::Column::ConsolidatedAt.is_null())
      .filter(episodic_memory::Column::ConversationId.eq(conversation_id))
      .order_by_asc(episodic_memory::Column::CreatedAt)
      .all(db)
      .await?;
    models.into_iter().map(Self::from_model).collect()
  }

  /// Mark the given episodes as consolidated (bulk update).
  pub async fn mark_consolidated<C: ConnectionTrait>(
    ids: &[Uuid],
    db: &C,
  ) -> Result<(), AppError> {
    use sea_orm::sea_query::Value as SeaValue;

    if ids.is_empty() {
      return Ok(());
    }

    let now: sea_orm::prelude::DateTimeWithTimeZone = Utc::now().into();

    // Bulk update: UPDATE episodic_memory SET consolidated_at = now WHERE id = ANY(ids)
    episodic_memory::Entity::update_many()
      .col_expr(
        episodic_memory::Column::ConsolidatedAt,
        Expr::value(now),
      )
      .filter(
        episodic_memory::Column::Id.is_in(
          ids.iter().copied().map(SeaValue::from).collect::<Vec<_>>()
        )
      )
      .exec(db)
      .await?;

    Ok(())
  }

  /// Retrieve episodic memories using hybrid BM25 + vector search with FSRS re-ranking.
  ///
  /// Only memories from the specified conversation are searched.
  pub async fn retrieve(
    query: &str,
    limit: u64,
    conversation_id: Uuid,
    db: &DatabaseConnection,
  ) -> Result<Vec<(Self, f64)>, AppError> {
    let query_embedding = embed(query).await?;
    let fsrs = FSRS::new(Some(&DEFAULT_PARAMETERS))?;

    let retrieve_sql = r"
    WITH
    fulltext AS (
      SELECT id, ROW_NUMBER() OVER (ORDER BY pdb.score(id) DESC) AS r
      FROM episodic_memory
      WHERE summary ||| $1 AND conversation_id = $2
      LIMIT $3
    ),
    semantic AS (
      SELECT id, ROW_NUMBER() OVER (ORDER BY embedding <#> $4) AS r
      FROM episodic_memory
      WHERE conversation_id = $2
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
      m.id,
      m.conversation_id,
      m.messages,
      m.title,
      m.summary,
      m.embedding,
      m.stability,
      m.difficulty,
      m.surprise,
      m.start_at,
      m.end_at,
      m.created_at,
      m.last_reviewed_at,
      r.score AS score
    FROM rrf_score r
    JOIN episodic_memory m USING (id)
    ORDER BY r.score DESC
    LIMIT $5;
    ";

    let params: Vec<sea_orm::Value> = vec![
      query.to_owned().into(),      // $1
      conversation_id.into(),       // $2
      100.into(),                   // $3: candidate limit
      query_embedding.into(),       // $4
      100.into(),                   // $5: final limit
    ];

    let retrieve_stmt = Statement::from_sql_and_values(DbBackend::Postgres, retrieve_sql, params);

    let rows = db.query_all_raw(retrieve_stmt).await?;
    let mut results = Vec::with_capacity(rows.len());
    let now = Utc::now();

    for row in rows {
      let model = episodic_memory::Model::from_query_result(&row, "")?;
      let rrf_score: f64 = row.try_get("", "score")?;
      let mem = Self::from_model(model)?;

      // FSRS re-ranking: multiply RRF score by retrievability
      // Use 0 if negative (clock skew) or unreasonably large
      let days_elapsed =
        u32::try_from((now - mem.last_reviewed_at).num_days().clamp(0, 365 * 100)).unwrap_or(0);
      let memory_state = MemoryState {
        stability: mem.stability,
        difficulty: mem.difficulty,
      };
      let retrievability =
        fsrs.current_retrievability(memory_state, days_elapsed, FSRS6_DEFAULT_DECAY);

      let final_score = rrf_score * f64::from(retrievability);

      results.push((mem, final_score));
    }

    // Re-sort by final score descending and truncate to requested limit
    results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    results.truncate(limit);

    Ok(results)
  }
}
