use std::fmt::Write;

use chrono::Utc;
use plastmem_ai::{
  ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
  ChatCompletionRequestUserMessage, embed_many, generate_object,
};
use plastmem_entities::semantic_memory;
use plastmem_shared::AppError;
use schemars::JsonSchema;
use sea_orm::{
  ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, DbBackend, EntityTrait,
  FromQueryResult, IntoActiveModel, QueryFilter, Statement, TransactionTrait,
  prelude::{Expr, PgVector},
  sea_query::{ArrayType, Value},
};
use serde::Deserialize;
use uuid::Uuid;

use crate::EpisodicMemory;

use super::SemanticMemory;

// ──────────────────────────────────────────────────
// LLM consolidation types
// ──────────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ConsolidationOutput {
  pub facts: Vec<ConsolidatedFact>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ConsolidatedFact {
  /// Action to take: "new", "reinforce", "update", or "invalidate"
  pub action: FactAction,
  /// ID of existing fact (required for reinforce, update, invalidate)
  pub existing_fact_id: Option<String>,
  pub subject: String,
  pub predicate: String,
  pub object: String,
  /// Natural language sentence describing the fact
  pub fact: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FactAction {
  /// A brand new fact not covered by existing knowledge
  New,
  /// An existing fact confirmed by new evidence
  Reinforce,
  /// An existing fact that needs modification
  Update,
  /// An existing fact contradicted by new evidence
  Invalidate,
}

// ──────────────────────────────────────────────────
// Consolidation prompt
// ──────────────────────────────────────────────────

const CONSOLIDATION_SYSTEM_PROMPT: &str = "\
You are performing memory consolidation — reviewing recent experiences \
against existing knowledge to update long-term memory.

For each piece of knowledge you identify, classify it:
1. \"new\": A fact not covered by existing knowledge.
2. \"reinforce\": An existing fact confirmed by new evidence. Include its ID in existing_fact_id.
3. \"update\": An existing fact that needs modification (e.g., a preference changed). Include its ID.
4. \"invalidate\": An existing fact contradicted by new evidence (e.g., moved cities). Include its ID.

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
3. Include a natural language `fact` sentence for each entry.
4. For new/update facts, preferences, habits, personal info, relationships, and significant events \
   are good candidates.
5. For behavioral rules, use subject = \"assistant\".
6. If no lasting facts can be extracted, return an empty `facts` array.
7. Multiple values for the same predicate can coexist (e.g., liking multiple things). \
   Only use \"invalidate\" when genuinely replaced (e.g., changed residence, corrected name).
8. Cross-reference across episodes: if multiple episodes mention the same fact, \
   that's stronger signal. Prefer one \"new\" entry over duplicate entries.

Predicate taxonomy (use these when applicable; create new ones if needed):

  Personal: likes, dislikes, prefers, lives_in, works_at, age_is, name_is
  Knowledge: is_interested_in, has_experience_with, knows_about
  Relational: communicate_in_style, relationship_is, has_shared_reference, has_routine
  Behavioral: should, should_not, should_when_[context], responds_to_[trigger]_with";

// ──────────────────────────────────────────────────
// Consolidation threshold
// ──────────────────────────────────────────────────

/// Minimum number of unconsolidated episodes to trigger consolidation.
pub const CONSOLIDATION_EPISODE_THRESHOLD: u64 = 3;

/// Surprise threshold for flashbulb memory — triggers immediate consolidation.
pub const FLASHBULB_SURPRISE_THRESHOLD: f32 = 0.85;

// ──────────────────────────────────────────────────
// Deduplication threshold
// ──────────────────────────────────────────────────

/// Cosine similarity threshold for embedding-based deduplication.
/// Facts with similarity above this are considered true duplicates.
const DEDUPE_THRESHOLD: f64 = 0.95;

// ──────────────────────────────────────────────────
// Helpers: find similar facts, append IDs, invalidate
// ──────────────────────────────────────────────────

/// Find existing active facts similar to the given embedding.
/// Returns all facts above the similarity threshold, ordered by similarity (highest first).
/// Only searches within the specified conversation.
async fn find_similar_facts<C: ConnectionTrait>(
  embedding: &PgVector,
  threshold: f64,
  conversation_id: Uuid,
  db: &C,
) -> Result<Vec<semantic_memory::Model>, AppError> {
  let sql = r"
  SELECT
    id, conversation_id, subject, predicate, object, fact, source_episodic_ids,
    valid_at, invalid_at, embedding, created_at,
    -(embedding <#> $1) AS similarity
  FROM semantic_memory
  WHERE conversation_id = $3
    AND invalid_at IS NULL
    AND -(embedding <#> $1) > $2
  ORDER BY similarity DESC
  LIMIT 5;
  ";

  let stmt = Statement::from_sql_and_values(
    DbBackend::Postgres,
    sql,
    vec![embedding.clone().into(), threshold.into(), conversation_id.into()],
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
async fn append_source_episodic_ids<C: ConnectionTrait>(
  fact_id: Uuid,
  existing_source_episodic_ids: &[Uuid],
  new_source_episodic_ids: &[Uuid],
  db: &C,
) -> Result<(), AppError> {
  // Filter out IDs that already exist
  let existing_set: std::collections::HashSet<_> = existing_source_episodic_ids.iter().collect();
  let ids_to_add: Vec<_> = new_source_episodic_ids
    .iter()
    .filter(|id| !existing_set.contains(id))
    .copied()
    .collect();

  if ids_to_add.is_empty() {
    return Ok(());
  }

  semantic_memory::Entity::update_many()
    .col_expr(
      semantic_memory::Column::SourceEpisodicIds,
      Expr::cust_with_values(
        "source_episodic_ids || ?::uuid[]",
        [Value::Array(
          ArrayType::Uuid,
          Some(Box::new(ids_to_add.into_iter().map(Into::into).collect())),
        )],
      ),
    )
    .filter(semantic_memory::Column::Id.eq(fact_id))
    .exec(db)
    .await?;

  Ok(())
}

/// Invalidate a fact by setting its `invalid_at` timestamp.
async fn invalidate_fact<C: ConnectionTrait>(fact_id: Uuid, db: &C) -> Result<(), AppError> {
  semantic_memory::Entity::update_many()
    .col_expr(
      semantic_memory::Column::InvalidAt,
      Expr::value(Utc::now()),
    )
    .filter(semantic_memory::Column::Id.eq(fact_id))
    .exec(db)
    .await?;

  Ok(())
}

// ──────────────────────────────────────────────────
// Load related existing facts
// ──────────────────────────────────────────────────

/// Retrieve existing active facts related to the given episodes.
/// Batches all episode embeddings in one API call, then searches per-episode to avoid
/// cross-episode semantic dilution. Results are deduplicated by fact ID.
/// Only searches within the specified conversation.
async fn load_related_facts(
  episodes: &[EpisodicMemory],
  limit: i64,
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
    let results =
      SemanticMemory::retrieve_by_vector(&ep.summary, embedding, limit, conversation_id, db)
        .await?;
    for (fact, _) in results {
      if seen_ids.insert(fact.id) {
        facts.push(fact);
      }
    }
  }

  Ok(facts)
}

// ──────────────────────────────────────────────────
// Action processing
// ──────────────────────────────────────────────────

/// Process a single consolidated fact action.
/// `valid_existing_ids` contains IDs that were actually presented to the LLM (for hallucination check).
async fn process_fact_action<C: ConnectionTrait>(
  fact: &ConsolidatedFact,
  embedding: PgVector,
  episode_ids: &[Uuid],
  valid_existing_ids: &[Uuid],
  conversation_id: Uuid,
  db: &C,
) -> Result<(), AppError> {
  // Validate existing_fact_id if provided
  let validated_existing_id = fact.existing_fact_id.as_deref()
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
      // Check for embedding-based duplicates before inserting
      let similar = find_similar_facts(&embedding, DEDUPE_THRESHOLD, conversation_id, db).await?;

      if let Some(existing) = similar.first() {
        tracing::debug!(
          existing_id = %existing.id,
          fact = %fact.fact,
          "Merging duplicate during consolidation"
        );
        append_source_episodic_ids(
          existing.id,
          &existing.source_episodic_ids,
          episode_ids,
          db,
        )
        .await?;
      } else {
        // Insert as new fact
        let id = Uuid::now_v7();
        let now = Utc::now();
        let model = semantic_memory::Model {
          id,
          conversation_id,
          subject: fact.subject.clone(),
          predicate: fact.predicate.clone(),
          object: fact.object.clone(),
          fact: fact.fact.clone(),
          source_episodic_ids: episode_ids.to_vec(),
          valid_at: now.into(),
          invalid_at: None,
          embedding,
          created_at: now.into(),
        };

        model.into_active_model().insert(db).await?;

        tracing::debug!(
          fact = %fact.fact,
          "Inserted new semantic fact via consolidation"
        );
      }
    }

    FactAction::Reinforce => {
      if let Some(existing_id) = validated_existing_id {
        // Find the existing fact and append source IDs
        if let Some(existing) = semantic_memory::Entity::find_by_id(existing_id)
          .one(db)
          .await?
        {
          append_source_episodic_ids(
            existing.id,
            &existing.source_episodic_ids,
            episode_ids,
            db,
          )
          .await?;

          tracing::debug!(
            existing_id = %existing_id,
            fact = %fact.fact,
            "Reinforced existing semantic fact"
          );
        }
      } else {
        tracing::warn!(fact = %fact.fact, "Reinforce action without valid existing_fact_id, skipping");
      }
    }

    FactAction::Update => {
      if let Some(existing_id) = validated_existing_id {
        // Invalidate old fact and insert updated version
        invalidate_fact(existing_id, db).await?;

        let id = Uuid::now_v7();
        let now = Utc::now();
        let model = semantic_memory::Model {
          id,
          conversation_id,
          subject: fact.subject.clone(),
          predicate: fact.predicate.clone(),
          object: fact.object.clone(),
          fact: fact.fact.clone(),
          source_episodic_ids: episode_ids.to_vec(),
          valid_at: now.into(),
          invalid_at: None,
          embedding,
          created_at: now.into(),
        };

        model.into_active_model().insert(db).await?;

        tracing::debug!(
          old_id = %existing_id,
          fact = %fact.fact,
          "Updated semantic fact (invalidated old, inserted new)"
        );
      } else {
        tracing::warn!(fact = %fact.fact, "Update action without valid existing_fact_id, skipping");
      }
    }

    FactAction::Invalidate => {
      if let Some(existing_id) = validated_existing_id {
        invalidate_fact(existing_id, db).await?;

        tracing::debug!(
          existing_id = %existing_id,
          fact = %fact.fact,
          "Invalidated semantic fact via consolidation"
        );
      } else {
        tracing::warn!(fact = %fact.fact, "Invalidate action without valid existing_fact_id, skipping");
      }
    }
  }

  Ok(())
}

/// Extract valid fact IDs from loaded existing facts for hallucination checking.
fn extract_valid_fact_ids(existing_facts: &[SemanticMemory]) -> Vec<Uuid> {
  existing_facts.iter().map(|f| f.id).collect()
}


// ──────────────────────────────────────────────────
// End-to-end consolidation pipeline
// ──────────────────────────────────────────────────

/// Process semantic consolidation for a batch of unconsolidated episodes.
///
/// This is the CLS-inspired offline replay pipeline:
/// 1. Load existing related facts (predict — "what do we already know?")
/// 2. Build consolidation prompt with existing facts + episode batch
/// 3. Single LLM call → `ConsolidationOutput` (calibrate — "what changed?")
/// 4. Process each result: insert/reinforce/update/invalidate
/// 5. Mark episodes as consolidated
///
/// All database operations are wrapped in a transaction for atomicity.
pub async fn process_consolidation(
  episodes: &[EpisodicMemory],
  db: &DatabaseConnection,
) -> Result<(), AppError> {
  if episodes.is_empty() {
    return Ok(());
  }

  let episode_ids: Vec<Uuid> = episodes.iter().map(|ep| ep.id).collect();

  // All episodes should belong to the same conversation
  let conversation_id = episodes[0].conversation_id;

  // 1. Load related existing facts (the "predict" step)
  let existing_facts = load_related_facts(episodes, 20, conversation_id, db).await?;
  let valid_fact_ids = extract_valid_fact_ids(&existing_facts);

  // 2. Build the consolidation prompt
  let mut existing_facts_section = String::new();
  if existing_facts.is_empty() {
    existing_facts_section.push_str("No existing knowledge yet.");
  } else {
    existing_facts_section.push_str("Current knowledge:\n");
    for fact in &existing_facts {
      let _ = writeln!(
        existing_facts_section,
        "- [ID: {}] ({}, {}, {}) — {}",
        fact.id, fact.subject, fact.predicate, fact.object, fact.fact
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

  // 3. LLM consolidation call
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
    // No facts to process, just mark episodes as consolidated
    let txn = db.begin().await?;
    EpisodicMemory::mark_consolidated(&episode_ids, &txn).await?;
    txn.commit().await?;
    return Ok(());
  }

  // Batch embed all fact sentences before opening a transaction
  let fact_texts: Vec<String> = output.facts.iter().map(|f| f.fact.clone()).collect();
  let embeddings = embed_many(&fact_texts).await?;

  // 4-6. All database mutations in a transaction (opened after embedding to keep it short)
  let txn = db.begin().await?;

  // Process each consolidated fact within the transaction
  for (fact, embedding) in output.facts.iter().zip(embeddings.into_iter()) {
    process_fact_action(fact, embedding, &episode_ids, &valid_fact_ids, conversation_id, &txn).await?;
  }

  // Mark episodes as consolidated
  EpisodicMemory::mark_consolidated(&episode_ids, &txn).await?;

  txn.commit().await?;
  Ok(())
}
