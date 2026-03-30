use std::{cmp::Ordering, collections::HashMap, time::Instant};

use anyhow::anyhow;
use apalis::prelude::Data;
use chrono::{DateTime, Utc};
use plastmem_ai::{
  ChatCompletionRequestMessage, embed, embed_many, generate_object, generate_text,
};
use plastmem_core::{EpisodicMemory, SemanticMemory};
use plastmem_entities::{episodic_memory, semantic_memory};
use plastmem_shared::AppError;
use schemars::JsonSchema;
use sea_orm::{
  ActiveModelTrait, ConnectionTrait, DatabaseConnection, DbBackend, EntityTrait, FromQueryResult,
  IntoActiveModel, Set, Statement, TransactionTrait, prelude::PgVector,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PredictCalibrateJob {
  pub conversation_id: Uuid,
  pub episode_id: Uuid,
  pub force: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SemanticActionOutput {
  actions: Vec<SemanticAction>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct SemanticAction {
  kind: SemanticActionKind,
  fact: String,
  category: String,
  target_fact_id: String,
  justification: String,
  confidence: f32,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
enum SemanticActionKind {
  New,
  Reinforce,
  Update,
  Invalidate,
}

const COLD_START_SYSTEM_PROMPT: &str = "\
You are a semantic memory consolidation system.

Read the episode and output semantic consolidation actions as strict JSON.

## Goal
Extract only high-value, durable semantic facts that should become active semantic memory.

## Rules
1. In cold start mode, you may only emit `new` actions.
2. Each action must use one of these categories exactly:
   identity, preference, interest, personality, relationship, experience, goal, guideline
3. Each fact must be atomic, self-contained, and useful for future retrieval.
4. Preserve exact names, titles, locations, diagnoses, products, model names, and other distinctive phrases.
5. Do not rewrite stable speaker labels into `User` or `Assistant` unless the source only uses those labels.
6. Skip temporary reactions, acknowledgements, and low-value contextual chatter.
7. Use present tense for persistent facts.
8. For `target_fact_id`, use an empty string in cold start mode.
9. For `confidence`, use a number between 0 and 1.

## Action semantics
- `new`: create a new active fact

## Examples
- GOOD new: fact='Sam works at ByteDance as a senior ML engineer', category='identity'
- GOOD new: fact='Sam prefers Rust over Python for systems programming', category='preference'
- BAD: fact='The user was happy', category='personality'
- BAD: fact='They talked about programming', category='interest'";

const PREDICTION_SYSTEM_PROMPT: &str = "\
You are performing the PREDICT phase of Predict-Calibrate Learning.

Given existing semantic knowledge about the conversation participants and an episode title, predict what the conversation content would be.

## Task
Generate a prediction of what the conversation SHOULD contain based on:
1. The episode title
2. Existing knowledge about the conversation participants

## Guidelines
1. Be specific and concrete in your prediction.
2. Reference relevant knowledge that informed your prediction.
3. If knowledge is insufficient, indicate what you cannot predict.
4. Preserve named participants and exact item names already present in the knowledge.
5. Do not normalize named speakers into `User` or `Assistant` unless the knowledge only provides generic role labels.
6. The prediction will be compared with actual conversation to identify knowledge gaps.";

const EXTRACT_FROM_COMPARISON_PROMPT: &str = "\
You are performing the CALIBRATE phase of Predict-Calibrate Learning.

You will receive:
- Existing active semantic facts, each with a stable `target_fact_id`
- A predicted episode description
- The actual messages

Output strict JSON containing semantic consolidation actions.

## Goal
Decide how semantic memory should change after seeing where the prediction was wrong, incomplete, or confirmed.

## Allowed actions
- `new`: create a new active fact not already covered by an existing fact
- `reinforce`: existing fact is confirmed as still accurate and should gain provenance
- `update`: existing fact is outdated or too imprecise; replace it with a new fact
- `invalidate`: existing fact is no longer true and should stop being active, with no replacement fact worth keeping

## Hard constraints
1. `reinforce`, `update`, and `invalidate` must reference a provided `target_fact_id`.
2. Never invent a target fact ID.
3. `update` must include the replacement `fact` and its final `category`.
4. `invalidate` must keep `fact` as an empty string.
5. `reinforce` should only be used when the existing fact remains semantically equivalent.
6. `new` should be used when no provided fact is an appropriate target.
7. Facts must be atomic, self-contained, persistent, and retrieval-friendly.
8. Preserve exact noun phrases when they matter for QA or retrieval.
9. Prefer fewer high-quality actions over many weak ones.
10. Use `confidence` between 0 and 1.

## Update guidance
- Use `update` for contradiction or material refinement, such as location changes, relationship changes, job changes, or replacing a vague fact with a more precise durable fact.
- Use `invalidate` when an existing fact is no longer true and no durable replacement is supported by the episode.
- Use `reinforce` when the episode simply confirms an existing fact.

## Categories
identity, preference, interest, personality, relationship, experience, goal, guideline";

const DEDUPE_THRESHOLD: f64 = 0.95;
const MAX_STATEMENTS_FOR_PREDICTION: usize = 10;
const MAX_GUIDELINES_FOR_PREDICTION: usize = 3;
const MAX_FACTS_FOR_ACTIONS: usize = 20;
const DUPLICATE_CANDIDATE_LIMIT: usize = 5;

pub async fn process_predict_calibrate(
  job: PredictCalibrateJob,
  db: Data<DatabaseConnection>,
) -> Result<(), AppError> {
  let db = &*db;

  let Some(episode) = EpisodicMemory::get(job.episode_id, db).await? else {
    tracing::warn!(
      episode_id = %job.episode_id,
      "Episode not found for predict-calibrate"
    );
    return Ok(());
  };

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

  let load_start = Instant::now();
  tracing::info!(episode_id = %episode.id, "Predict-Calibrate stage start: load_related_facts");
  let existing_facts = load_related_facts(
    &episode,
    db,
    i64::try_from(MAX_FACTS_FOR_ACTIONS)
      .map_err(|_| anyhow!("MAX_FACTS_FOR_ACTIONS must fit in i64"))?,
  )
  .await?;
  tracing::info!(
    episode_id = %episode.id,
    elapsed_ms = load_start.elapsed().as_millis(),
    facts_found = existing_facts.len(),
    "Predict-Calibrate stage done: load_related_facts"
  );

  let extraction_start = Instant::now();
  let actions = if existing_facts.is_empty() {
    tracing::info!(episode_id = %episode.id, "No existing knowledge, using cold start mode");
    cold_start_extraction(&episode).await?
  } else {
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
    action_count = actions.len(),
    "Predict-Calibrate stage done: extract_knowledge"
  );

  if actions.is_empty() {
    tracing::debug!("No semantic actions extracted, marking episode as consolidated");
    mark_consolidated(job.episode_id, db).await?;
    return Ok(());
  }

  let consolidate_start = Instant::now();
  tracing::info!(
    episode_id = %episode.id,
    action_count = actions.len(),
    "Predict-Calibrate stage start: consolidate_actions"
  );
  consolidate_actions(&actions, &episode, &existing_facts, db).await?;
  tracing::info!(
    episode_id = %episode.id,
    elapsed_ms = consolidate_start.elapsed().as_millis(),
    "Predict-Calibrate stage done: consolidate_actions"
  );

  mark_consolidated(job.episode_id, db).await?;

  tracing::info!(
    episode_id = %job.episode_id,
    "Predict-Calibrate Learning completed"
  );

  Ok(())
}

fn format_messages(episode: &EpisodicMemory) -> String {
  episode
    .messages
    .iter()
    .enumerate()
    .map(|(i, m)| format!("Message {} [{}]: {}", i + 1, m.role, m.content))
    .collect::<Vec<_>>()
    .join("\n")
}

async fn cold_start_extraction(episode: &EpisodicMemory) -> Result<Vec<SemanticAction>, AppError> {
  let user_content = format!(
    "Episode Title: {}\nEpisode Content: {}\n\nMessages:\n{}",
    episode.title,
    episode.content,
    format_messages(episode)
  );

  tracing::debug!(episode_id = %episode.id, "Cold-start extraction");
  let generation_start = Instant::now();
  tracing::info!(episode_id = %episode.id, "Predict-Calibrate stage start: cold_start_generate");

  let output = generate_object::<SemanticActionOutput>(
    vec![
      ChatCompletionRequestMessage::System(COLD_START_SYSTEM_PROMPT.into()),
      ChatCompletionRequestMessage::User(user_content.into()),
    ],
    "pcl_cold_start".to_owned(),
    Some("Generate semantic memory creation actions from the first episode".to_owned()),
  )
  .await?;

  tracing::info!(
    episode_id = %episode.id,
    elapsed_ms = generation_start.elapsed().as_millis(),
    action_count = output.actions.len(),
    "Predict-Calibrate stage done: cold_start_generate"
  );

  Ok(normalize_actions(output.actions))
}

async fn predict_calibrate_extraction(
  episode: &EpisodicMemory,
  existing_facts: &[(SemanticMemory, f64)],
) -> Result<Vec<SemanticAction>, AppError> {
  let prediction_facts = select_relevant_facts(existing_facts);
  let action_candidates = select_action_candidates(existing_facts);

  let predict_start = Instant::now();
  tracing::info!(
    episode_id = %episode.id,
    fact_count = prediction_facts.len(),
    "Predict-Calibrate stage start: predict"
  );
  let prediction = predict_episode(&episode.title, &prediction_facts).await?;
  tracing::info!(
    episode_id = %episode.id,
    elapsed_ms = predict_start.elapsed().as_millis(),
    prediction_len = prediction.len(),
    "Predict-Calibrate stage done: predict"
  );

  let existing_facts_text = format_existing_facts_for_prompt(&action_candidates);
  let user_content = format!(
    "## Episode Title\n{}\n\n## Existing Active Facts\n{}\n\n## PREDICTED Content\n{}\n\n## ACTUAL Messages\n{}",
    episode.title,
    existing_facts_text,
    prediction,
    format_messages(episode)
  );

  tracing::debug!(episode_id = %episode.id, "Extracting semantic actions from gaps");
  let calibrate_start = Instant::now();
  tracing::info!(episode_id = %episode.id, "Predict-Calibrate stage start: calibrate");

  let output = generate_object::<SemanticActionOutput>(
    vec![
      ChatCompletionRequestMessage::System(EXTRACT_FROM_COMPARISON_PROMPT.into()),
      ChatCompletionRequestMessage::User(user_content.into()),
    ],
    "pcl_calibrate".to_owned(),
    Some("Generate semantic memory update actions from prediction-vs-actual comparison".to_owned()),
  )
  .await?;

  tracing::info!(
    episode_id = %episode.id,
    elapsed_ms = calibrate_start.elapsed().as_millis(),
    action_count = output.actions.len(),
    "Predict-Calibrate stage done: calibrate"
  );

  Ok(normalize_actions(output.actions))
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

async fn consolidate_actions(
  actions: &[SemanticAction],
  source: &EpisodicMemory,
  existing_facts: &[(SemanticMemory, f64)],
  db: &DatabaseConnection,
) -> Result<(), AppError> {
  let normalized_actions = normalize_actions(actions.to_vec());
  let statements_to_embed = normalized_actions
    .iter()
    .filter(|action| {
      matches!(
        action.kind,
        SemanticActionKind::New | SemanticActionKind::Update
      )
    })
    .map(|action| action.fact.clone())
    .collect::<Vec<_>>();

  let embeddings = if statements_to_embed.is_empty() {
    Vec::new()
  } else {
    let embed_start = Instant::now();
    tracing::info!(
      episode_id = %source.id,
      statement_count = statements_to_embed.len(),
      "Predict-Calibrate stage start: embed_many"
    );
    let embeddings = embed_many(&statements_to_embed).await?;
    tracing::info!(
      episode_id = %source.id,
      elapsed_ms = embed_start.elapsed().as_millis(),
      embedding_count = embeddings.len(),
      "Predict-Calibrate stage done: embed_many"
    );
    embeddings
  };

  let active_map = existing_facts
    .iter()
    .map(|(memory, _)| (memory.id.to_string(), memory.clone()))
    .collect::<HashMap<_, _>>();

  let tx = db.begin().await?;
  let mut current_active_map = active_map.clone();
  let mut embedding_iter = embeddings.into_iter();

  for action in normalized_actions {
    match action.kind {
      SemanticActionKind::Reinforce => {
        if let Some(target) = resolve_active_target(&action, &current_active_map) {
          reinforce_existing(&target, source.id, &tx).await?;
        } else {
          tracing::warn!(
            episode_id = %source.id,
            target_fact_id = %action.target_fact_id,
            "Skipping reinforce with unknown or inactive target"
          );
        }
      }
      SemanticActionKind::Invalidate => {
        if let Some(target) = resolve_active_target(&action, &current_active_map) {
          invalidate_existing(target.id, &tx).await?;
          current_active_map.remove(&action.target_fact_id);
        } else {
          tracing::warn!(
            episode_id = %source.id,
            target_fact_id = %action.target_fact_id,
            "Skipping invalidate with unknown or inactive target"
          );
        }
      }
      SemanticActionKind::New => {
        let embedding = embedding_iter
          .next()
          .ok_or_else(|| AppError::new(anyhow!("Missing embedding for semantic action")))?;
        let duplicate = find_duplicate_in_scope(
          &embedding,
          source.conversation_id,
          &current_active_map,
          &tx,
          None,
        )
        .await?;

        if let Some(existing) = duplicate {
          let existing = SemanticMemory::from_model(existing);
          reinforce_existing(&existing, source.id, &tx).await?;
          current_active_map.insert(existing.id.to_string(), existing);
        } else {
          let inserted =
            insert_new_fact(&action.fact, &action.category, embedding, source, &tx).await?;
          current_active_map.insert(inserted.id.to_string(), inserted);
        }
      }
      SemanticActionKind::Update => {
        let embedding = embedding_iter
          .next()
          .ok_or_else(|| AppError::new(anyhow!("Missing embedding for semantic action")))?;

        let Some(target) = resolve_active_target(&action, &current_active_map) else {
          let duplicate = find_duplicate_in_scope(
            &embedding,
            source.conversation_id,
            &current_active_map,
            &tx,
            None,
          )
          .await?;
          if let Some(existing) = duplicate {
            let existing = SemanticMemory::from_model(existing);
            reinforce_existing(&existing, source.id, &tx).await?;
            current_active_map.insert(existing.id.to_string(), existing);
          } else {
            let inserted =
              insert_new_fact(&action.fact, &action.category, embedding, source, &tx).await?;
            current_active_map.insert(inserted.id.to_string(), inserted);
          }
          tracing::warn!(
            episode_id = %source.id,
            target_fact_id = %action.target_fact_id,
            "Downgraded update to new because target was unavailable"
          );
          continue;
        };

        let duplicate = find_duplicate_in_scope(
          &embedding,
          source.conversation_id,
          &current_active_map,
          &tx,
          Some(target.id),
        )
        .await?;

        invalidate_existing(target.id, &tx).await?;
        current_active_map.remove(&action.target_fact_id);

        if let Some(existing) = duplicate {
          let existing = SemanticMemory::from_model(existing);
          reinforce_existing(&existing, source.id, &tx).await?;
          current_active_map.insert(existing.id.to_string(), existing);
        } else {
          let inserted =
            insert_new_fact(&action.fact, &action.category, embedding, source, &tx).await?;
          current_active_map.insert(inserted.id.to_string(), inserted);
        }
      }
    }
  }

  tx.commit().await?;
  Ok(())
}

fn normalize_actions(actions: Vec<SemanticAction>) -> Vec<SemanticAction> {
  let mut targeted: HashMap<String, SemanticAction> = HashMap::new();
  let mut untargeted = Vec::new();

  for mut action in actions {
    action.fact = action.fact.trim().to_owned();
    action.category = normalize_category(&action.category);
    action.target_fact_id = action.target_fact_id.trim().to_owned();
    action.justification = action.justification.trim().to_owned();
    action.confidence = action.confidence.clamp(0.0, 1.0);

    if action.kind == SemanticActionKind::Invalidate {
      action.fact.clear();
    }

    if matches!(
      action.kind,
      SemanticActionKind::New | SemanticActionKind::Update
    ) && action.fact.is_empty()
    {
      continue;
    }

    if matches!(
      action.kind,
      SemanticActionKind::Reinforce | SemanticActionKind::Update | SemanticActionKind::Invalidate
    ) {
      if action.target_fact_id.is_empty() {
        if action.kind == SemanticActionKind::Update {
          action.kind = SemanticActionKind::New;
        } else {
          continue;
        }
      }
      let key = action.target_fact_id.clone();
      match targeted.get(&key) {
        Some(existing) if action_priority(&action.kind) > action_priority(&existing.kind) => {
          targeted.insert(key, action);
        }
        None => {
          targeted.insert(key, action);
        }
        _ => {}
      }
    } else {
      untargeted.push(action);
    }
  }

  untargeted.extend(targeted.into_values());
  untargeted
}

fn action_priority(kind: &SemanticActionKind) -> u8 {
  match kind {
    SemanticActionKind::Reinforce => 1,
    SemanticActionKind::Invalidate => 2,
    SemanticActionKind::Update => 3,
    SemanticActionKind::New => 0,
  }
}

fn normalize_category(raw: &str) -> String {
  match raw.trim().to_ascii_lowercase().as_str() {
    "identity" => "identity".to_owned(),
    "preference" => "preference".to_owned(),
    "interest" => "interest".to_owned(),
    "personality" => "personality".to_owned(),
    "relationship" => "relationship".to_owned(),
    "experience" => "experience".to_owned(),
    "goal" => "goal".to_owned(),
    "guideline" => "guideline".to_owned(),
    _ => "identity".to_owned(),
  }
}

fn select_relevant_facts(facts: &[(SemanticMemory, f64)]) -> Vec<&SemanticMemory> {
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

fn select_action_candidates(facts: &[(SemanticMemory, f64)]) -> Vec<&SemanticMemory> {
  facts
    .iter()
    .take(MAX_FACTS_FOR_ACTIONS)
    .map(|(fact, _)| fact)
    .collect()
}

fn format_existing_facts_for_prompt(facts: &[&SemanticMemory]) -> String {
  if facts.is_empty() {
    return "None".to_owned();
  }

  facts
    .iter()
    .map(|fact| {
      format!(
        "- target_fact_id={}\n  category={}\n  fact={}\n  valid_at={}\n  source_count={}",
        fact.id,
        fact.category,
        fact.fact,
        fact.valid_at.to_rfc3339(),
        fact.source_episodic_ids.len()
      )
    })
    .collect::<Vec<_>>()
    .join("\n")
}

async fn load_related_facts(
  episode: &EpisodicMemory,
  db: &DatabaseConnection,
  limit: i64,
) -> Result<Vec<(SemanticMemory, f64)>, AppError> {
  let content_embedding = embed(&episode.content).await?;
  let results = SemanticMemory::retrieve_by_embedding(
    &episode.content,
    content_embedding,
    limit,
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

fn resolve_active_target(
  action: &SemanticAction,
  active_map: &HashMap<String, SemanticMemory>,
) -> Option<SemanticMemory> {
  active_map.get(&action.target_fact_id).cloned()
}

async fn find_duplicate_in_scope<C: ConnectionTrait>(
  embedding: &PgVector,
  conversation_id: Uuid,
  active_map: &HashMap<String, SemanticMemory>,
  db: &C,
  exclude_id: Option<Uuid>,
) -> Result<Option<semantic_memory::Model>, AppError> {
  let mut similar =
    find_similar_facts(embedding, DEDUPE_THRESHOLD, conversation_id, db, exclude_id).await?;
  similar.sort_by(|a, b| {
    let a_current = active_map.contains_key(&a.id.to_string());
    let b_current = active_map.contains_key(&b.id.to_string());
    match (a_current, b_current) {
      (true, false) => Ordering::Less,
      (false, true) => Ordering::Greater,
      _ => Ordering::Equal,
    }
  });
  Ok(
    similar
      .into_iter()
      .find(|model| active_map.contains_key(&model.id.to_string())),
  )
}

async fn reinforce_existing<C: ConnectionTrait>(
  existing: &SemanticMemory,
  episode_id: Uuid,
  db: &C,
) -> Result<(), AppError> {
  if existing.source_episodic_ids.contains(&episode_id) {
    return Ok(());
  }

  let mut source_ids = existing.source_episodic_ids.clone();
  source_ids.push(episode_id);

  semantic_memory::Entity::update(semantic_memory::ActiveModel {
    id: Set(existing.id),
    source_episodic_ids: Set(source_ids),
    ..Default::default()
  })
  .exec(db)
  .await?;

  Ok(())
}

async fn invalidate_existing<C: ConnectionTrait>(id: Uuid, db: &C) -> Result<(), AppError> {
  let now: sea_orm::prelude::DateTimeWithTimeZone = Utc::now().into();
  semantic_memory::Entity::update(semantic_memory::ActiveModel {
    id: Set(id),
    invalid_at: Set(Some(now)),
    ..Default::default()
  })
  .exec(db)
  .await?;
  Ok(())
}

async fn insert_new_fact<C: ConnectionTrait>(
  statement: &str,
  category: &str,
  embedding: PgVector,
  source: &EpisodicMemory,
  db: &C,
) -> Result<SemanticMemory, AppError> {
  let now = Utc::now();
  let valid_at = effective_valid_at(source);

  let model = semantic_memory::Model {
    id: Uuid::now_v7(),
    conversation_id: source.conversation_id,
    category: normalize_category(category),
    fact: statement.to_string(),
    source_episodic_ids: vec![source.id],
    valid_at: valid_at.into(),
    invalid_at: None,
    embedding,
    created_at: now.into(),
  };

  let inserted = model.into_active_model().insert(db).await?;
  Ok(SemanticMemory::from_model(inserted))
}

fn effective_valid_at(source: &EpisodicMemory) -> DateTime<Utc> {
  if source.end_at >= source.start_at {
    source.end_at
  } else {
    source.start_at
  }
}

async fn find_similar_facts<C: ConnectionTrait>(
  embedding: &PgVector,
  threshold: f64,
  conversation_id: Uuid,
  db: &C,
  exclude_id: Option<Uuid>,
) -> Result<Vec<semantic_memory::Model>, AppError> {
  let sql = r"
  SELECT id, conversation_id, category, fact, source_episodic_ids,
    valid_at, invalid_at, embedding, created_at, -(embedding <#> $1) AS similarity
  FROM semantic_memory
  WHERE conversation_id = $2
    AND invalid_at IS NULL
    AND ($4::uuid IS NULL OR id <> $4)
    AND -(embedding <#> $1) > $3
  ORDER BY similarity DESC
  LIMIT 5";

  let stmt = Statement::from_sql_and_values(
    DbBackend::Postgres,
    sql,
    vec![
      embedding.clone().into(),
      conversation_id.into(),
      threshold.into(),
      exclude_id.into(),
    ],
  );

  let rows = db.query_all_raw(stmt).await?;
  let mut results = Vec::with_capacity(rows.len());
  for row in rows {
    results.push(semantic_memory::Model::from_query_result(&row, "")?);
  }
  results.truncate(DUPLICATE_CANDIDATE_LIMIT);
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

#[cfg(test)]
mod tests {
  use super::*;

  fn action(
    kind: SemanticActionKind,
    target_fact_id: &str,
    fact: &str,
    category: &str,
  ) -> SemanticAction {
    SemanticAction {
      kind,
      fact: fact.to_owned(),
      category: category.to_owned(),
      target_fact_id: target_fact_id.to_owned(),
      justification: String::new(),
      confidence: 0.8,
    }
  }

  #[test]
  fn normalize_actions_promotes_update_without_target_to_new() {
    let actions = normalize_actions(vec![action(
      SemanticActionKind::Update,
      "",
      "User lives in Tokyo",
      "identity",
    )]);

    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].kind, SemanticActionKind::New);
    assert_eq!(actions[0].fact, "User lives in Tokyo");
  }

  #[test]
  fn normalize_actions_keeps_highest_priority_targeted_action() {
    let actions = normalize_actions(vec![
      action(
        SemanticActionKind::Reinforce,
        "fact-1",
        "User lives in Osaka",
        "identity",
      ),
      action(SemanticActionKind::Invalidate, "fact-1", "", "identity"),
      action(
        SemanticActionKind::Update,
        "fact-1",
        "User lives in Tokyo",
        "identity",
      ),
    ]);

    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].kind, SemanticActionKind::Update);
    assert_eq!(actions[0].fact, "User lives in Tokyo");
  }

  #[test]
  fn normalize_category_falls_back_to_identity() {
    assert_eq!(normalize_category("unknown"), "identity");
    assert_eq!(normalize_category("guideline"), "guideline");
  }
}
