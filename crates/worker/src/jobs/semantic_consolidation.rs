use std::fmt::Write as FmtWrite;

use apalis::prelude::Data;
use chrono::Utc;
use plastmem_ai::{
  ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
  ChatCompletionRequestUserMessage, embed_many, generate_object,
};
use plastmem_core::{EpisodicMemory, SemanticMemory};

const CONSOLIDATION_EPISODE_THRESHOLD: u64 = 3;
use plastmem_entities::{episodic_memory, semantic_memory};
use plastmem_shared::AppError;
use schemars::JsonSchema;
use sea_orm::{
  ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, DbBackend, EntityTrait,
  FromQueryResult, IntoActiveModel, QueryFilter, QueryOrder, Statement, TransactionTrait,
  prelude::{Expr, PgVector},
  sea_query::Value as SeaValue,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ──────────────────────────────────────────────────
// Job definition
// ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticConsolidationJob {
  pub conversation_id: Uuid,
  /// If true, consolidate even if below the episode threshold (e.g., flashbulb trigger).
  pub force: bool,
}

// ──────────────────────────────────────────────────
// LLM consolidation types
// ──────────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
struct ConsolidationOutput {
  facts: Vec<ConsolidatedFact>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ConsolidatedFact {
  action: FactAction,
  existing_fact_id: Option<String>,
  category: String,
  fact: String,
  keywords: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum FactAction {
  New,
  Reinforce,
  Update,
  Invalidate,
}

// ──────────────────────────────────────────────────
// Consolidation prompt
// ──────────────────────────────────────────────────

const CONSOLIDATION_SYSTEM_PROMPT: &str = "\
You are performing memory consolidation — reviewing recent conversation episodes \
against existing semantic memory to extract and update long-term knowledge.

## Memory Categories
- identity    : User's personal facts — name, age, location, occupation, background
- preference  : What the user likes, dislikes, or prefers
- interest    : Topics, hobbies, subjects the user is curious about or passionate about
- personality : Personality traits, emotional patterns, communication style
- relationship: Dynamics between user and AI — inside jokes, rituals, communication patterns
- experience  : Significant events or experiences the user has had or discussed
- goal        : User's goals, plans, aspirations, ongoing projects
- guideline   : How the AI should behave — response style, topics to avoid, conditional rules

## Actions
- \"new\"       : A lasting fact not covered by existing knowledge
- \"reinforce\" : Existing fact confirmed by new evidence — include existing_fact_id
- \"update\"    : Existing fact needs modification — include existing_fact_id (old will be invalidated)
- \"invalidate\": Existing fact contradicted by new evidence — include existing_fact_id

## Rules
1. Only extract durable facts. Skip transient states (\"I'm tired today\" → ignore).
2. Write facts as concise natural language sentences (\"User lives in Tokyo\").
3. keywords: 2-5 entity names and key nouns from the fact, e.g. [\"user\", \"Tokyo\"].
4. For guideline: describe specific AI behavior rules (\"AI should use casual tone\").
5. Multiple episodes confirming the same fact → one \"new\", not duplicates.
6. New episode contradicts existing fact → \"update\" or \"invalidate\", not \"new\".
7. If nothing lasting found → return empty facts array.";

// ──────────────────────────────────────────────────
// Consolidation helpers
// ──────────────────────────────────────────────────

const DEDUPE_THRESHOLD: f64 = 0.95;
const DUPLICATE_CANDIDATE_LIMIT: i64 = 5;
const RELATED_FACTS_LIMIT: i64 = 20;

async fn find_similar_facts<C: ConnectionTrait>(
  embedding: &PgVector,
  threshold: f64,
  conversation_id: Uuid,
  db: &C,
) -> Result<Vec<semantic_memory::Model>, AppError> {
  let sql = r"
  SELECT
    id, conversation_id, category, fact, keywords, source_episodic_ids,
    valid_at, invalid_at, embedding, created_at,
    -(embedding <#> $1) AS similarity
  FROM semantic_memory
  WHERE conversation_id = $3
    AND invalid_at IS NULL
    AND -(embedding <#> $1) > $2
  ORDER BY similarity DESC
  LIMIT $4;
  ";

  let stmt = Statement::from_sql_and_values(
    DbBackend::Postgres,
    sql,
    vec![
      embedding.clone().into(),
      threshold.into(),
      conversation_id.into(),
      DUPLICATE_CANDIDATE_LIMIT.into(),
    ],
  );

  let rows = db.query_all_raw(stmt).await?;
  let mut results = Vec::with_capacity(rows.len());
  for row in rows {
    results.push(semantic_memory::Model::from_query_result(&row, "")?);
  }
  Ok(results)
}

async fn append_source_episodic_ids<C: ConnectionTrait>(
  fact_id: Uuid,
  existing_source_episodic_ids: &[Uuid],
  new_source_episodic_ids: &[Uuid],
  db: &C,
) -> Result<(), AppError> {
  let existing_set: std::collections::HashSet<_> = existing_source_episodic_ids.iter().collect();
  let ids_to_add: Vec<_> = new_source_episodic_ids
    .iter()
    .filter(|id| !existing_set.contains(id))
    .copied()
    .collect();

  if ids_to_add.is_empty() {
    return Ok(());
  }

  let uuid_list = ids_to_add.iter().map(|id| format!("'{id}'")).collect::<Vec<_>>().join(",");
  let sql = format!(
    "UPDATE semantic_memory SET source_episodic_ids = source_episodic_ids || ARRAY[{uuid_list}]::uuid[] WHERE id = $1"
  );
  db.execute_raw(Statement::from_sql_and_values(DbBackend::Postgres, &sql, [fact_id.into()]))
    .await?;

  Ok(())
}

async fn invalidate_fact<C: ConnectionTrait>(fact_id: Uuid, db: &C) -> Result<(), AppError> {
  semantic_memory::Entity::update_many()
    .col_expr(semantic_memory::Column::InvalidAt, Expr::value(Utc::now()))
    .filter(semantic_memory::Column::Id.eq(fact_id))
    .exec(db)
    .await?;
  Ok(())
}

async fn load_related_facts(
  episodes: &[EpisodicMemory],
  conversation_id: Uuid,
  db: &DatabaseConnection,
) -> Result<Vec<SemanticMemory>, AppError> {
  if episodes.is_empty() {
    return Ok(Vec::new());
  }

  let summaries: Vec<String> = episodes.iter().map(|ep| ep.summary.clone()).collect();
  let embeddings = embed_many(&summaries).await?;

  let mut seen_ids = std::collections::HashSet::new();
  let mut facts = Vec::new();

  for (ep, embedding) in episodes.iter().zip(embeddings.into_iter()) {
    let results = SemanticMemory::retrieve_by_embedding(
      &ep.summary,
      embedding,
      RELATED_FACTS_LIMIT,
      conversation_id,
      db,
      None,
    )
    .await?;
    for (fact, _) in results {
      if seen_ids.insert(fact.id) {
        facts.push(fact);
      }
    }
  }

  Ok(facts)
}

async fn process_fact_action<C: ConnectionTrait>(
  fact: &ConsolidatedFact,
  embedding: PgVector,
  episode_ids: &[Uuid],
  valid_existing_ids: &[Uuid],
  conversation_id: Uuid,
  db: &C,
) -> Result<(), AppError> {
  let validated_existing_id = fact
    .existing_fact_id
    .as_deref()
    .and_then(|s| Uuid::parse_str(s).ok())
    .filter(|id| valid_existing_ids.contains(id));

  if fact.existing_fact_id.is_some() && validated_existing_id.is_none() {
    tracing::warn!(
      fact = %fact.fact,
      existing_fact_id = ?fact.existing_fact_id,
      "LLM returned invalid or hallucinated fact ID, treating as 'new'"
    );
  }

  match fact.action {
    FactAction::New => {
      let similar = find_similar_facts(&embedding, DEDUPE_THRESHOLD, conversation_id, db).await?;
      if let Some(existing) = similar.first() {
        tracing::debug!(existing_id = %existing.id, fact = %fact.fact, "Merging duplicate during consolidation");
        append_source_episodic_ids(existing.id, &existing.source_episodic_ids, episode_ids, db)
          .await?;
      } else {
        insert_semantic_fact(fact, embedding, episode_ids, conversation_id, db).await?;
        tracing::debug!(fact = %fact.fact, "Inserted new semantic fact via consolidation");
      }
    }

    FactAction::Reinforce => {
      if let Some(existing_id) = validated_existing_id {
        if let Some(existing) = semantic_memory::Entity::find_by_id(existing_id).one(db).await? {
          append_source_episodic_ids(existing.id, &existing.source_episodic_ids, episode_ids, db)
            .await?;
          tracing::debug!(existing_id = %existing_id, fact = %fact.fact, "Reinforced existing semantic fact");
        }
      } else {
        tracing::warn!(fact = %fact.fact, "Reinforce action without valid existing_fact_id, skipping");
      }
    }

    FactAction::Update => {
      if let Some(existing_id) = validated_existing_id {
        invalidate_fact(existing_id, db).await?;
        insert_semantic_fact(fact, embedding, episode_ids, conversation_id, db).await?;
        tracing::debug!(old_id = %existing_id, fact = %fact.fact, "Updated semantic fact");
      } else {
        tracing::warn!(fact = %fact.fact, "Update action without valid existing_fact_id, skipping");
      }
    }

    FactAction::Invalidate => {
      if let Some(existing_id) = validated_existing_id {
        invalidate_fact(existing_id, db).await?;
        tracing::debug!(existing_id = %existing_id, fact = %fact.fact, "Invalidated semantic fact");
      } else {
        tracing::warn!(fact = %fact.fact, "Invalidate action without valid existing_fact_id, skipping");
      }
    }
  }

  Ok(())
}

// ──────────────────────────────────────────────────
// Job processing
// ──────────────────────────────────────────────────

pub async fn process_semantic_consolidation(
  job: SemanticConsolidationJob,
  db: Data<DatabaseConnection>,
) -> Result<(), AppError> {
  let db = &*db;

  let episodes = fetch_unconsolidated(job.conversation_id, db).await?;

  if episodes.is_empty() {
    tracing::debug!(conversation_id = %job.conversation_id, "No unconsolidated episodes, skipping consolidation");
    return Ok(());
  }

  if !job.force && (episodes.len() as u64) < CONSOLIDATION_EPISODE_THRESHOLD {
    tracing::debug!(
      conversation_id = %job.conversation_id,
      episodes = episodes.len(),
      threshold = CONSOLIDATION_EPISODE_THRESHOLD,
      "Below consolidation threshold, skipping"
    );
    return Ok(());
  }

  tracing::info!(
    conversation_id = %job.conversation_id,
    episodes = episodes.len(),
    force = job.force,
    "Processing semantic consolidation"
  );

  let episode_ids: Vec<Uuid> = episodes.iter().map(|ep| ep.id).collect();
  let conversation_id = episodes[0].conversation_id;

  let existing_facts = load_related_facts(&episodes, conversation_id, db).await?;
  let valid_fact_ids: Vec<Uuid> = existing_facts.iter().map(|f| f.id).collect();

  let mut existing_facts_section = String::new();
  if existing_facts.is_empty() {
    existing_facts_section.push_str("No existing knowledge yet.");
  } else {
    existing_facts_section.push_str("Current knowledge:\n");
    for fact in &existing_facts {
      let _ = writeln!(
        existing_facts_section,
        "- [ID: {}] [{}] {}",
        fact.id, fact.category, fact.fact
      );
    }
  }

  let mut episodes_section = String::new();
  for (i, ep) in episodes.iter().enumerate() {
    let _ = writeln!(episodes_section, "\n--- Episode {} ---", i + 1);
    let _ = writeln!(episodes_section, "Summary: {}", ep.summary);
    let _ = writeln!(episodes_section, "Surprise: {:.2}", ep.surprise);
    let _ = writeln!(episodes_section, "Conversation:");
    for msg in &ep.messages {
      let _ = writeln!(episodes_section, "  {msg}");
    }
  }

  let user_content = format!(
    "== Existing Knowledge ==\n{existing_facts_section}\n\n\
     == Recent Experiences (oldest first) ==\n{episodes_section}"
  );

  let system = ChatCompletionRequestSystemMessage::from(CONSOLIDATION_SYSTEM_PROMPT);
  let user = ChatCompletionRequestUserMessage::from(user_content);

  let output = generate_object::<ConsolidationOutput>(
    vec![
      ChatCompletionRequestMessage::System(system),
      ChatCompletionRequestMessage::User(user),
    ],
    "semantic_consolidation".to_owned(),
    Some("Consolidate recent experiences into long-term semantic memory".to_owned()),
  )
  .await?;

  tracing::info!(
    episodes = episodes.len(),
    facts_count = output.facts.len(),
    "Semantic consolidation completed"
  );

  if output.facts.is_empty() {
    let txn = db.begin().await?;
    mark_consolidated(&episode_ids, &txn).await?;
    txn.commit().await?;
    return Ok(());
  }

  let embed_inputs: Vec<String> = output
    .facts
    .iter()
    .map(|f| format!("{}: {} {}", f.category, f.fact, f.keywords.join(" ")))
    .collect();
  let embeddings = embed_many(&embed_inputs).await?;

  let txn = db.begin().await?;
  for (fact, embedding) in output.facts.iter().zip(embeddings.into_iter()) {
    process_fact_action(fact, embedding, &episode_ids, &valid_fact_ids, conversation_id, &txn)
      .await?;
  }
  mark_consolidated(&episode_ids, &txn).await?;
  txn.commit().await?;

  Ok(())
}

async fn insert_semantic_fact<C: ConnectionTrait>(
  fact: &ConsolidatedFact,
  embedding: PgVector,
  episode_ids: &[Uuid],
  conversation_id: Uuid,
  db: &C,
) -> Result<(), AppError> {
  let now = Utc::now();
  semantic_memory::Model {
    id: Uuid::now_v7(),
    conversation_id,
    category: fact.category.clone(),
    fact: fact.fact.clone(),
    keywords: fact.keywords.clone(),
    source_episodic_ids: episode_ids.to_vec(),
    valid_at: now.into(),
    invalid_at: None,
    embedding,
    created_at: now.into(),
  }
  .into_active_model()
  .insert(db)
  .await?;
  Ok(())
}

async fn fetch_unconsolidated(
  conversation_id: Uuid,
  db: &DatabaseConnection,
) -> Result<Vec<EpisodicMemory>, AppError> {
  let models = episodic_memory::Entity::find()
    .filter(episodic_memory::Column::ConsolidatedAt.is_null())
    .filter(episodic_memory::Column::ConversationId.eq(conversation_id))
    .order_by_asc(episodic_memory::Column::CreatedAt)
    .all(db)
    .await?;
  models.into_iter().map(EpisodicMemory::from_model).collect()
}

async fn mark_consolidated<C: ConnectionTrait>(ids: &[Uuid], db: &C) -> Result<(), AppError> {
  if ids.is_empty() {
    return Ok(());
  }
  let now: sea_orm::prelude::DateTimeWithTimeZone = Utc::now().into();
  episodic_memory::Entity::update_many()
    .col_expr(episodic_memory::Column::ConsolidatedAt, Expr::value(now))
    .filter(episodic_memory::Column::Id.is_in(ids.iter().copied().map(SeaValue::from)))
    .exec(db)
    .await?;
  Ok(())
}
