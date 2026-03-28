use std::time::Instant;

use apalis::prelude::Data;
use chrono::Utc;
use plastmem_ai::{ChatCompletionRequestMessage, embed, embed_many, generate_object, generate_text};
use plastmem_core::{EpisodicMemory, SemanticMemory};

use plastmem_entities::{episodic_memory, semantic_memory};
use plastmem_shared::AppError;
use schemars::JsonSchema;
use sea_orm::{
  ActiveModelTrait, ConnectionTrait, DatabaseConnection, DbBackend, EntityTrait, FromQueryResult,
  IntoActiveModel, Set, Statement, prelude::PgVector,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ──────────────────────────────────────────────────
// Job definition
// ──────────────────────────────────────────────────

/// Predict-Calibrate Job - Real-time knowledge learning from single episode
///
/// Implements Nemori's Predict-Calibrate Learning principle:
/// 1. PREDICT: Generate prediction from existing semantic knowledge
/// 2. CALIBRATE: Compare prediction with actual episode, extract knowledge from gaps
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PredictCalibrateJob {
  pub conversation_id: Uuid,
  /// The specific episode to learn from
  pub episode_id: Uuid,
  /// If true, force processing (e.g., for flashbulb memories)
  pub force: bool,
}

// ──────────────────────────────────────────────────
// LLM Types
// ──────────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
struct KnowledgeExtractionOutput {
  statements: Vec<String>,
}

// ──────────────────────────────────────────────────
// Prompts
// ──────────────────────────────────────────────────

/// Cold-start extraction prompt - when no existing knowledge
const COLD_START_SYSTEM_PROMPT: &str = "\
You are a knowledge extraction specialist. Extract HIGH-VALUE, PERSISTENT knowledge statements from the following episode.

## CRITICAL: Focus on HIGH-VALUE Knowledge Only

Extract ONLY knowledge that passes these criteria:
- **Persistence Test**: Will this still be true in 6 months?
- **Specificity Test**: Does it contain concrete, searchable information?
- **Utility Test**: Can this help predict future needs, preferences, or behavior of the conversation participants?
- **Independence Test**: Can this be understood without the conversation context?

## HIGH-VALUE Knowledge Categories (EXTRACT THESE):
1. **Identity & Background**: Names, professions, companies, education
2. **Persistent Preferences**: Favorite books/movies/tools, long-term likes/dislikes
3. **Technical Details**: Technologies, versions, methodologies, architectures
4. **Relationships**: Family, colleagues, team members, mentors
5. **Goals & Plans**: Career objectives, learning goals, project plans
6. **Beliefs & Values**: Principles, philosophies, strong opinions
7. **Habits & Patterns**: Regular activities, workflows, schedules

## LOW-VALUE Knowledge (SKIP THESE):
- Temporary emotions or reactions
- Single conversation acknowledgments
- Vague statements without specifics
- Context-dependent information

## Guidelines:
1. Each statement should be self-contained and atomic
2. Include ALL specific details (names, versions, titles)
3. Use present tense for persistent facts
4. Focus on facts that help understand the user long-term
5. DO NOT include time/date information in the statement
6. If stable speaker names or labels appear in the episode, use those exact names or labels in the statement. Do NOT rewrite them as `User` or `Assistant`.
7. Preserve exact noun phrases verbatim when they matter for retrieval or QA. Do NOT generalize products, devices, diagnoses, holidays, locations, titles of works, model names, or other distinctive phrases into broader descriptions.
8. Prefer the shortest exact supported phrase from the source messages.
9. Quality over quantity - fewer valuable statements are better

## Examples:
GOOD: \"Sam's favorite movie is 'The Godfather'\"
GOOD: \"Evan works at ByteDance as a senior ML engineer\"
GOOD: \"Evan uses a fitness tracker\"
GOOD: \"Sam was diagnosed with gastritis\"
BAD: \"The user thanked the assistant\"
BAD: \"The user was happy about the response\"
BAD: \"Evan uses a health tracking tool\"
BAD: \"Sam had a digestive issue\"";

/// Prediction prompt - generate expected episode content from existing knowledge
const PREDICTION_SYSTEM_PROMPT: &str = "\
You are performing the PREDICT phase of Predict-Calibrate Learning.

Given existing semantic knowledge about the conversation participants and an episode title, predict what the conversation content would be.

## Task
Generate a prediction of what the conversation SHOULD contain based on:
1. The episode title
2. Existing knowledge about the conversation participants

Your prediction should be a natural description of what you would expect to see in this conversation if your existing knowledge is accurate and complete.

## Guidelines:
1. Be specific and concrete in your prediction
2. Reference relevant knowledge that informed your prediction
3. If knowledge is insufficient, indicate what you cannot predict
4. Preserve named participants and exact item names already present in the knowledge.
5. Do not normalize named speakers into `User` or `Assistant` unless the knowledge only provides generic role labels.
6. The prediction will be compared with actual conversation to identify knowledge gaps";

/// Knowledge extraction from comparison prompt
const EXTRACT_FROM_COMPARISON_PROMPT: &str = "\
You are performing the CALIBRATE phase of Predict-Calibrate Learning.

Compare the predicted conversation with the actual messages and extract HIGH-VALUE knowledge from the differences.

## Task
Identify where the prediction was WRONG or INCOMPLETE - these gaps reveal new knowledge to learn.

## CRITICAL: Focus on HIGH-VALUE Knowledge Only

Extract ONLY knowledge that passes these criteria:
- **Persistence Test**: Will this still be true in 6 months?
- **Specificity Test**: Does it contain concrete, searchable information?
- **Utility Test**: Can this help predict future needs, preferences, or behavior of the conversation participants?
- **Surprise Test**: Was this unexpected given existing knowledge?

## Gap Types to Extract:
1. **Missing Knowledge**: Could not have predicted this from existing knowledge
2. **Contradiction**: Existing knowledge was wrong or outdated
3. **Refinement**: Existing knowledge was incomplete or vague

## Guidelines:
1. Extract knowledge that explains WHY the prediction was wrong
2. Focus on facts that would improve future predictions
3. Each statement should be self-contained and atomic
4. Include ALL specific details (names, versions, titles)
5. Use present tense for persistent facts
6. If stable speaker names or labels appear in the messages, use those exact names or labels in the statement. Do NOT rewrite them as `User` or `Assistant`.
7. Preserve exact noun phrases verbatim when they matter for retrieval or QA. Do NOT generalize products, devices, diagnoses, holidays, locations, titles of works, model names, or other distinctive phrases into broader descriptions.
8. Prefer the shortest exact supported phrase from the source messages.
9. Quality over quantity

## Examples:
GOOD: \"Sam prefers Rust over Python for systems programming\"
GOOD: \"Evan is learning Japanese for an upcoming trip to Tokyo\"
GOOD: \"Evan got married during the Christmas season\"
GOOD: \"Sam is going kayaking at Lake Tahoe\"
BAD: \"The user disagreed with the prediction\"
BAD: \"The conversation was about programming\"
BAD: \"Evan got married during the winter holiday season\"
BAD: \"Sam is doing an outdoor activity by a lake\"";

// ──────────────────────────────────────────────────
// Constants
// ──────────────────────────────────────────────────

const DEDUPE_THRESHOLD: f64 = 0.95;
const MAX_STATEMENTS_FOR_PREDICTION: usize = 10;
const MAX_GUIDELINES_FOR_PREDICTION: usize = 3;

// ──────────────────────────────────────────────────
// Main Job Handler
// ──────────────────────────────────────────────────

pub async fn process_predict_calibrate(
  job: PredictCalibrateJob,
  db: Data<DatabaseConnection>,
) -> Result<(), AppError> {
  let db = &*db;

  // Load the target episode
  let Some(episode) = EpisodicMemory::get(job.episode_id, db).await? else {
    tracing::warn!(
      episode_id = %job.episode_id,
      "Episode not found for predict-calibrate"
    );
    return Ok(());
  };

  // Skip if already consolidated
  if episode.consolidated_at.is_some() {
    tracing::debug!(
      episode_id = %job.episode_id,
      "Episode already consolidated, skipping"
    );
    return Ok(());
  }

  tracing::info!(
    conversation_id = %job.conversation_id,
    episode_id = %job.episode_id,
    episode_title = %episode.title,
    "Starting Predict-Calibrate Learning"
  );

  // Load existing semantic memories for this conversation
  let load_start = Instant::now();
  tracing::info!(episode_id = %episode.id, "Predict-Calibrate stage start: load_related_facts");
  let existing_facts = load_related_facts(&episode, db).await?;
  tracing::info!(
    episode_id = %episode.id,
    elapsed_ms = load_start.elapsed().as_millis(),
    facts_found = existing_facts.len(),
    "Predict-Calibrate stage done: load_related_facts"
  );

  // Extract knowledge using Predict-Calibrate Learning
  let extraction_start = Instant::now();
  let statements = if existing_facts.is_empty() {
    // Cold start: no existing knowledge to predict from
    tracing::info!(episode_id = %episode.id, "No existing knowledge, using cold start mode");
    cold_start_extraction(&episode).await?
  } else {
    // Predict-Calibrate: use stratified selection to pick relevant facts
    // Guidelines are prioritized to ensure AI behavior rules are considered
    tracing::debug!(
      episode_id = %episode.id,
      facts_found = existing_facts.len(),
      "Using Predict-Calibrate with existing knowledge"
    );
    predict_calibrate_extraction(&episode, &existing_facts).await?
  };
  tracing::info!(
    episode_id = %episode.id,
    elapsed_ms = extraction_start.elapsed().as_millis(),
    statement_count = statements.len(),
    "Predict-Calibrate stage done: extract_knowledge"
  );

  if statements.is_empty() {
    tracing::debug!("No knowledge extracted, marking episode as consolidated");
    mark_consolidated(job.episode_id, db).await?;
    return Ok(());
  }

  tracing::info!(
    statement_count = statements.len(),
    "Knowledge statements extracted"
  );

  // Consolidate extracted statements with existing knowledge
  let consolidate_start = Instant::now();
  tracing::info!(episode_id = %episode.id, statement_count = statements.len(), "Predict-Calibrate stage start: consolidate_statements");
  consolidate_statements(&statements, &episode, db).await?;
  tracing::info!(
    episode_id = %episode.id,
    elapsed_ms = consolidate_start.elapsed().as_millis(),
    "Predict-Calibrate stage done: consolidate_statements"
  );

  // Mark episode as consolidated
  mark_consolidated(job.episode_id, db).await?;

  tracing::info!(
    episode_id = %job.episode_id,
    "Predict-Calibrate Learning completed"
  );

  Ok(())
}

// ──────────────────────────────────────────────────
// Cold Start Extraction
// ──────────────────────────────────────────────────

// Helper: format episode messages for prompts
fn format_messages(episode: &EpisodicMemory) -> String {
  episode
    .messages
    .iter()
    .enumerate()
    .map(|(i, m)| format!("Message {} [{}]: {}", i + 1, m.role, m.content))
    .collect::<Vec<_>>()
    .join("\n")
}

async fn cold_start_extraction(episode: &EpisodicMemory) -> Result<Vec<String>, AppError> {
  let user_content = format!(
    "Episode Title: {}\nEpisode Content: {}\n\nMessages:\n{}",
    episode.title,
    episode.content,
    format_messages(episode)
  );

  tracing::debug!(episode_id = %episode.id, "Cold-start extraction");
  let generation_start = Instant::now();
  tracing::info!(episode_id = %episode.id, "Predict-Calibrate stage start: cold_start_generate");

  let output = generate_object::<KnowledgeExtractionOutput>(
    vec![
      ChatCompletionRequestMessage::System(COLD_START_SYSTEM_PROMPT.into()),
      ChatCompletionRequestMessage::User(user_content.into()),
    ],
    "pcl_cold_start".to_owned(),
    Some("Extract high-value knowledge from first episode".to_owned()),
  )
  .await?;

  tracing::info!(
    episode_id = %episode.id,
    elapsed_ms = generation_start.elapsed().as_millis(),
    statement_count = output.statements.len(),
    "Predict-Calibrate stage done: cold_start_generate"
  );

  Ok(output.statements)
}

// ──────────────────────────────────────────────────
// Predict-Calibrate Extraction
// ──────────────────────────────────────────────────

async fn predict_calibrate_extraction(
  episode: &EpisodicMemory,
  existing_facts: &[(SemanticMemory, f64)],
) -> Result<Vec<String>, AppError> {
  // Step 1: PREDICT - Generate prediction from stratified facts (guidelines prioritized)
  let facts = select_relevant_facts(existing_facts);
  let predict_start = Instant::now();
  tracing::info!(
    episode_id = %episode.id,
    fact_count = facts.len(),
    "Predict-Calibrate stage start: predict"
  );
  let prediction = predict_episode(&episode.title, &facts).await?;
  tracing::info!(
    episode_id = %episode.id,
    elapsed_ms = predict_start.elapsed().as_millis(),
    prediction_len = prediction.len(),
    "Predict-Calibrate stage done: predict"
  );

  // Step 2: CALIBRATE - Compare prediction with actual messages
  let user_content = format!(
    "## Episode Title\n{}\n\n## PREDICTED Content\n{}\n\n## ACTUAL Messages\n{}",
    episode.title,
    prediction,
    format_messages(episode)
  );

  tracing::debug!(episode_id = %episode.id, "Extracting knowledge from gaps");
  let calibrate_start = Instant::now();
  tracing::info!(episode_id = %episode.id, "Predict-Calibrate stage start: calibrate");

  let output = generate_object::<KnowledgeExtractionOutput>(
    vec![
      ChatCompletionRequestMessage::System(EXTRACT_FROM_COMPARISON_PROMPT.into()),
      ChatCompletionRequestMessage::User(user_content.into()),
    ],
    "pcl_calibrate".to_owned(),
    Some("Extract knowledge from prediction-actual comparison".to_owned()),
  )
  .await?;

  tracing::info!(
    episode_id = %episode.id,
    elapsed_ms = calibrate_start.elapsed().as_millis(),
    statement_count = output.statements.len(),
    "Predict-Calibrate stage done: calibrate"
  );

  Ok(output.statements)
}

async fn predict_episode(title: &str, facts: &[&SemanticMemory]) -> Result<String, AppError> {
  if facts.is_empty() {
    return Ok(format!("No knowledge available to predict '{title}'."));
  }

  let facts_text = facts
    .iter()
    .map(|f| format!("- [{}] {}", f.category, f.fact))
    .collect::<Vec<_>>()
    .join("\n");
  let user_content = format!("Episode Title: {title}\n\nExisting Knowledge:\n{facts_text}");

  generate_text(vec![
    ChatCompletionRequestMessage::System(PREDICTION_SYSTEM_PROMPT.into()),
    ChatCompletionRequestMessage::User(user_content.into()),
  ])
  .await
}

// ──────────────────────────────────────────────────
// Consolidation
// ──────────────────────────────────────────────────

async fn consolidate_statements(
  statements: &[String],
  source: &EpisodicMemory,
  db: &DatabaseConnection,
) -> Result<(), AppError> {
  let embed_start = Instant::now();
  tracing::info!(
    episode_id = %source.id,
    statement_count = statements.len(),
    "Predict-Calibrate stage start: embed_many"
  );
  let embeddings = embed_many(statements).await?;
  tracing::info!(
    episode_id = %source.id,
    elapsed_ms = embed_start.elapsed().as_millis(),
    embedding_count = embeddings.len(),
    "Predict-Calibrate stage done: embed_many"
  );

  for (stmt, emb) in statements.iter().zip(embeddings) {
    if let Some(existing) = find_duplicate(&emb, source.conversation_id, db).await? {
      merge_with_existing(&existing, source.id, db).await?;
      tracing::debug!(existing_id = %existing.id, "Merged duplicate fact");
    } else {
      insert_new_fact(stmt, emb, source, db).await?;
      tracing::debug!("Inserted new semantic fact");
    }
  }
  Ok(())
}

async fn find_duplicate(
  embedding: &PgVector,
  conversation_id: Uuid,
  db: &DatabaseConnection,
) -> Result<Option<semantic_memory::Model>, AppError> {
  let similar = find_similar_facts(embedding, DEDUPE_THRESHOLD, conversation_id, db).await?;
  Ok(similar.into_iter().next())
}

async fn merge_with_existing(
  existing: &semantic_memory::Model,
  episode_id: Uuid,
  db: &DatabaseConnection,
) -> Result<(), AppError> {
  if existing.source_episodic_ids.contains(&episode_id) {
    return Ok(());
  }

  let sql = "UPDATE semantic_memory SET source_episodic_ids = source_episodic_ids || ARRAY[$1]::uuid[] WHERE id = $2";
  db.execute_raw(Statement::from_sql_and_values(
    DbBackend::Postgres,
    sql,
    vec![episode_id.into(), existing.id.into()],
  ))
  .await?;

  Ok(())
}

async fn insert_new_fact(
  statement: &str,
  embedding: PgVector,
  source: &EpisodicMemory,
  db: &DatabaseConnection,
) -> Result<(), AppError> {
  let now = Utc::now();

  semantic_memory::Model {
    id: Uuid::now_v7(),
    conversation_id: source.conversation_id,
    category: infer_category(statement).to_string(),
    fact: statement.to_string(),
    source_episodic_ids: vec![source.id],
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

fn infer_category(statement: &str) -> &'static str {
  let s = statement.to_lowercase();
  if s.contains("prefer") || s.contains("like") || s.contains("enjoy") {
    "preference"
  } else if s.contains("work") || s.contains("job") || s.contains("career") {
    "identity"
  } else if s.contains("goal") || s.contains("plan") {
    "goal"
  } else if s.contains("family") || s.contains("friend") || s.contains("relationship") {
    "relationship"
  } else if s.contains("learn") || s.contains("study") {
    "interest"
  } else if s.contains("should") || s.contains("avoid") {
    "guideline"
  } else {
    "identity"
  }
}

// ──────────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────────

fn select_relevant_facts(facts: &[(SemanticMemory, f64)]) -> Vec<&SemanticMemory> {
  // Stratified: guidelines first (affect AI style), then other high-relevance facts
  let guidelines: Vec<_> = facts
    .iter()
    .filter(|(f, _)| f.is_behavioral())
    .take(MAX_GUIDELINES_FOR_PREDICTION)
    .map(|(f, _)| f)
    .collect();

  let remaining = MAX_STATEMENTS_FOR_PREDICTION.saturating_sub(guidelines.len());
  let others: Vec<_> = facts
    .iter()
    .filter(|(f, _)| !f.is_behavioral())
    .take(remaining)
    .map(|(f, _)| f)
    .collect();

  let guideline_count = guidelines.len();
  let mut selected = guidelines;
  selected.extend(others);

  tracing::debug!(
    total = selected.len(),
    guidelines = guideline_count,
    "Selected facts with guideline priority"
  );
  selected
}

async fn load_related_facts(
  episode: &EpisodicMemory,
  db: &DatabaseConnection,
) -> Result<Vec<(SemanticMemory, f64)>, AppError> {
  let content_embedding = embed(&episode.content).await?;
  let results = SemanticMemory::retrieve_by_embedding(
    &episode.content,
    content_embedding,
    i64::try_from(MAX_STATEMENTS_FOR_PREDICTION)
      .map_err(|_| anyhow::anyhow!("MAX_STATEMENTS_FOR_PREDICTION must fit in i64"))?,
    episode.conversation_id,
    db,
    None,
  )
  .await?;

  let max_score = results.first().map_or(0.0, |(_, s)| *s);
  tracing::debug!(
    episode_id = %episode.id,
    facts_found = results.len(),
    max_score = ?max_score,
    "Retrieved relevant semantic facts"
  );

  Ok(results)
}

async fn find_similar_facts<C: ConnectionTrait>(
  embedding: &PgVector,
  threshold: f64,
  conversation_id: Uuid,
  db: &C,
) -> Result<Vec<semantic_memory::Model>, AppError> {
  let sql = r"
  SELECT id, conversation_id, category, fact, source_episodic_ids,
    valid_at, invalid_at, embedding, created_at, -(embedding <#> $1) AS similarity
  FROM semantic_memory
  WHERE conversation_id = $2 AND invalid_at IS NULL AND -(embedding <#> $1) > $3
  ORDER BY similarity DESC LIMIT 5";

  let stmt = Statement::from_sql_and_values(
    DbBackend::Postgres,
    sql,
    vec![
      embedding.clone().into(),
      conversation_id.into(),
      threshold.into(),
    ],
  );

  let rows = db.query_all_raw(stmt).await?;
  let mut results = Vec::with_capacity(rows.len());
  for row in rows {
    results.push(semantic_memory::Model::from_query_result(&row, "")?);
  }
  Ok(results)
}

async fn mark_consolidated<C: ConnectionTrait>(episode_id: Uuid, db: &C) -> Result<(), AppError> {
  let now: sea_orm::prelude::DateTimeWithTimeZone = Utc::now().into();
  episodic_memory::Entity::update(episodic_memory::ActiveModel {
    id: Set(episode_id),
    consolidated_at: Set(Some(now)),
    ..Default::default()
  })
  .exec(db)
  .await?;
  Ok(())
}
