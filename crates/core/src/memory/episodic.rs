use crate::Message;
use chrono::{DateTime, Utc};
use plast_mem_db_schema::episodic_memory;
use plast_mem_llm::embed;
use plast_mem_shared::AppError;
use sea_orm::{
  ConnectionTrait, DatabaseConnection, DbBackend, FromQueryResult, Statement, prelude::PgVector,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EpisodicMemory {
  pub id: Uuid,
  pub conversation_id: Uuid,
  pub messages: Vec<Message>,
  pub content: String,
  pub embedding: PgVector,
  pub start_at: DateTime<Utc>,
  pub end_at: DateTime<Utc>,
  pub created_at: DateTime<Utc>,
  pub last_reviewed_at: DateTime<Utc>,
}

impl EpisodicMemory {
  // pub async fn new(conversation_id: Uuid, messages: Vec<Message>) -> Result<Self, AppError> {
  //   let now = Utc::now();
  //   let id = Uuid::now_v7();
  //   let start_at = messages.first().map(|m| m.timestamp).unwrap_or(now);
  //   let end_at = messages.last().map(|m| m.timestamp).unwrap_or(now);

  //   let input_messages = messages
  //     .iter()
  //     .map(|m| InputMessage {
  //       role: match m.role {
  //         MessageRole::User => Role::User,
  //         MessageRole::Assistant => Role::Assistant,
  //       },
  //       content: m.content.clone(),
  //     })
  //     .collect::<Vec<_>>();

  //   let content = summarize_messages(&input_messages).await?;
  //   let embedding = embed(&content).await?;

  //   Ok(Self {
  //     id,
  //     conversation_id,
  //     messages,
  //     content,
  //     embedding,
  //     start_at,
  //     end_at,
  //     created_at: now,
  //     last_reviewed_at: now,
  //   })
  // }

  pub fn from_model(model: episodic_memory::Model) -> Result<Self, AppError> {
    Ok(Self {
      id: model.id,
      conversation_id: model.conversation_id,
      messages: serde_json::from_value(model.messages)?,
      content: model.content,
      embedding: model.embedding,
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
        limit.into(),
        query_embedding.clone().into(),
        limit.into(),
      ],
    );

    let rows = db.query_all_raw(retrieve_stmt).await?;
    let mut results = Vec::with_capacity(rows.len());

    for row in rows {
      let model = episodic_memory::Model::from_query_result(&row, "")?;
      let score = row.try_get("", "score")?;
      let mem = EpisodicMemory::from_model(model)?;
      results.push((mem, score));
    }

    Ok(results)
  }
}
