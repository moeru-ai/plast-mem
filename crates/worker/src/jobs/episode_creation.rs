use std::fmt::Write;

use apalis::prelude::{Data, TaskSink};
use apalis_postgres::PostgresStorage;
use chrono::{DateTime, Datelike, Timelike, Utc};
use fsrs::{DEFAULT_PARAMETERS, FSRS};
use plastmem_ai::{
  ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
  ChatCompletionRequestUserMessage, embed, generate_object,
};
use plastmem_core::{EpisodeSpan, get_episode_span, get_messages_in_range};
use plastmem_entities::{EpisodeClassification, episodic_memory};
use plastmem_shared::{AppError, Message};
use schemars::JsonSchema;
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::PredictCalibrateJob;

const DESIRED_RETENTION: f32 = 0.9;
const EPISODE_CREATION_JOB_NAMESPACE: Uuid =
  Uuid::from_u128(0x7b70f7c6_0c6d_4bb9_b0d2_9386445a6104);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpisodeCreationJob {
  pub conversation_id: Uuid,
  pub start_seq: i64,
  pub end_seq: i64,
}

impl EpisodeCreationJob {
  pub const fn from_span(span: &EpisodeSpan) -> Self {
    Self {
      conversation_id: span.conversation_id,
      start_seq: span.start_seq,
      end_seq: span.end_seq,
    }
  }

  fn deterministic_episode_id(&self) -> Uuid {
    Uuid::new_v5(
      &EPISODE_CREATION_JOB_NAMESPACE,
      format!(
        "{}:{}:{}",
        self.conversation_id, self.start_seq, self.end_seq
      )
      .as_bytes(),
    )
  }
}

#[derive(Debug, Deserialize, JsonSchema)]
struct EpisodeTitleOutput {
  title: String,
}

#[derive(Debug, Clone)]
struct RenderedEpisodeLine {
  line_index: usize,
  timestamp: DateTime<Utc>,
  role: String,
  content: String,
}

#[derive(Debug, Clone)]
struct TimeAnchorCandidateLine {
  line_index: usize,
  timestamp: DateTime<Utc>,
  role: String,
  content: String,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
enum TimeAnchorPrecision {
  Time,
  Day,
  Week,
  Month,
  Year,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct TimeAnchorOutput {
  insertions: Vec<TimeAnchorInsertion>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct TimeAnchorInsertion {
  line_index: u32,
  exact_text: String,
  anchor_text: String,
  precision: TimeAnchorPrecision,
}

const EPISODE_TITLE_SYSTEM_PROMPT: &str = r"
You are naming one conversation segment for episodic memory retrieval.
Return only JSON with `title`.

Requirements:
1. The title must be concise, descriptive, and easy to search.
2. Keep it within 10-20 words and name the main topic, activity, or event.
3. Preserve names, places, products, and distinctive wording when they help retrieval.
4. Do not invent facts or generalize away the concrete topic.
";

const EPISODE_TIME_ANCHOR_SYSTEM_PROMPT: &str = r"
You are adding grounded time anchors to existing conversation lines.
Return only JSON with `insertions`.

Each insertion must contain:
- `line_index`: the exact line index shown in the input
- `exact_text`: copy the original time phrase exactly as it appears in the line content
- `anchor_text`: only the grounded parenthetical text to append, without parentheses
- `precision`: one of `time`, `day`, `week`, `month`, or `year`

Rules:
1. Do not rewrite, summarize, reorder, or delete any line.
2. Only add anchors for time expressions that already exist in the content.
3. Use `spoken_at` as the reference point for resolving relative time expressions.
4. If a line contains multiple time expressions that can be grounded with high confidence, return one insertion for each of them.
5. If a phrase cannot be resolved cleanly from `spoken_at`, omit the insertion entirely.
6. `exact_text` must copy the full original time phrase exactly as it appears in the line. Do not return only a fragment of the phrase.
7. `anchor_text` must add grounded calendar information. Do not repeat, paraphrase, abstract, or generalize the original phrase.
8. The anchor precision must never be more specific than the original phrase supports.
9. Match the original phrase's granularity:
   - `last year` -> `2022` with precision `year`
   - `last month` -> `July 2023` with precision `month`
   - `the previous weekend` -> `June 17-18, 2023` with precision `week`
   - `next Tuesday` -> `May 16, 2023` with precision `day`
   - `next Monday at 10:30 AM` -> `May 15, 2023 10:30 AM` with precision `time`
10. Preserve the original phrase in the line and resolve it inline after that phrase.
11. Do not modify non-time text.
12. Do not include parentheses inside `anchor_text`.
";

// ──────────────────────────────────────────────────
// Entry
// ──────────────────────────────────────────────────

pub async fn process_episode_creation(
  job: EpisodeCreationJob,
  db: Data<DatabaseConnection>,
  predict_storage: Data<PostgresStorage<PredictCalibrateJob>>,
) -> Result<(), AppError> {
  let db = &*db;

  let Some(span) = try_load_current_span(&job, db).await? else {
    return Ok(());
  };

  let episode_id = job.deterministic_episode_id();
  let already_consolidated = try_ensure_episode_exists(episode_id, &span, db).await?;

  try_enqueue_predict_calibrate_if_needed(
    span.conversation_id,
    episode_id,
    &span.classification,
    already_consolidated,
    &predict_storage,
  )
  .await?;

  Ok(())
}

// ──────────────────────────────────────────────────
// Episode Lifecycle
// ──────────────────────────────────────────────────

async fn try_load_current_span(
  job: &EpisodeCreationJob,
  db: &DatabaseConnection,
) -> Result<Option<EpisodeSpan>, AppError> {
  let Some(span) = get_episode_span(job.conversation_id, job.start_seq, db).await? else {
    tracing::debug!(
      conversation_id = %job.conversation_id,
      start_seq = job.start_seq,
      end_seq = job.end_seq,
      "Skipping stale episode creation job"
    );
    return Ok(None);
  };

  if span.end_seq != job.end_seq {
    tracing::debug!(
      conversation_id = %job.conversation_id,
      start_seq = job.start_seq,
      queued_end_seq = job.end_seq,
      actual_end_seq = span.end_seq,
      "Skipping mismatched episode creation job"
    );
    return Ok(None);
  }

  Ok(Some(span))
}

// Retries may observe an episode record that already exists even though the
// previous attempt never reached downstream consolidation scheduling.
async fn try_ensure_episode_exists(
  episode_id: Uuid,
  span: &EpisodeSpan,
  db: &DatabaseConnection,
) -> Result<bool, AppError> {
  if let Some(existing_episode) = episodic_memory::Entity::find_by_id(episode_id)
    .one(db)
    .await?
  {
    return Ok(existing_episode.consolidated_at.is_some());
  }

  let messages = load_episode_source_messages(span, db).await?;
  create_episode_record(episode_id, span, &messages, db).await?;
  Ok(false)
}

fn should_enqueue_predict_calibrate(classification: &EpisodeClassification) -> bool {
  matches!(classification, EpisodeClassification::Informative)
}

// Skip fully consolidated episodes, but still re-enqueue unfinished ones so
// episode creation retries can resume the downstream pipeline.
async fn try_enqueue_predict_calibrate_if_needed(
  conversation_id: Uuid,
  episode_id: Uuid,
  classification: &EpisodeClassification,
  already_consolidated: bool,
  predict_storage: &PostgresStorage<PredictCalibrateJob>,
) -> Result<(), AppError> {
  if !should_enqueue_predict_calibrate(classification) {
    return Ok(());
  }
  if already_consolidated {
    return Ok(());
  }

  let mut storage = predict_storage.clone();
  storage
    .push(PredictCalibrateJob {
      conversation_id,
      episode_id,
      force: false,
    })
    .await?;

  Ok(())
}

// ──────────────────────────────────────────────────
// Episode Persistence
// ──────────────────────────────────────────────────

async fn load_episode_source_messages(
  span: &EpisodeSpan,
  db: &DatabaseConnection,
) -> Result<Vec<Message>, AppError> {
  let conversation_messages =
    get_messages_in_range(span.conversation_id, span.start_seq, span.end_seq, db).await?;
  if conversation_messages.is_empty() {
    return Err(AppError::new(anyhow::anyhow!(
      "Episode span has no backing messages"
    )));
  }

  Ok(
    conversation_messages
      .iter()
      .map(plastmem_core::ConversationMessage::to_message)
      .collect(),
  )
}

async fn create_episode_record(
  episode_id: Uuid,
  span: &EpisodeSpan,
  messages: &[Message],
  db: &DatabaseConnection,
) -> Result<(), AppError> {
  let (title, content) = generate_episode_artifacts(messages).await?;
  let embedding = embed(&content).await?;

  let fsrs = FSRS::new(Some(&DEFAULT_PARAMETERS))?;
  let initial_states = fsrs.next_states(None, DESIRED_RETENTION, 0)?;
  let initial_state = initial_states.good.memory;
  let now = Utc::now();
  let start_at = messages.first().map_or(now, |message| message.timestamp);
  let end_at = messages.last().map_or(now, |message| message.timestamp);

  episodic_memory::ActiveModel {
    id: Set(episode_id),
    conversation_id: Set(span.conversation_id),
    messages: Set(serde_json::to_value(messages.to_vec())?),
    content: Set(content),
    embedding: Set(embedding),
    title: Set(title),
    stability: Set(initial_state.stability),
    difficulty: Set(initial_state.difficulty),
    surprise: Set(0.0),
    classification: Set(Some(span.classification.clone())),
    start_at: Set(start_at.into()),
    end_at: Set(end_at.into()),
    created_at: Set(now.into()),
    last_reviewed_at: Set(now.into()),
    consolidated_at: Set(None),
  }
  .insert(db)
  .await?;

  Ok(())
}

// ──────────────────────────────────────────────────
// Artifact Generation
// ──────────────────────────────────────────────────

// Render a deterministic transcript first, then let the LLM add grounded time
// anchors before generating the retrieval title.
async fn generate_episode_artifacts(messages: &[Message]) -> Result<(String, String), AppError> {
  let mut lines = render_episode_lines(messages);
  try_anchor_episode_lines(&mut lines).await;
  let content = render_episode_content(&lines);
  let title = generate_episode_title(messages, &content).await?;
  Ok((title, content))
}

async fn generate_episode_title(messages: &[Message], content: &str) -> Result<String, AppError> {
  let system = ChatCompletionRequestSystemMessage::from(EPISODE_TITLE_SYSTEM_PROMPT.trim());
  let user = ChatCompletionRequestUserMessage::from(format!(
    "Episode content:\n{}\n\nSource messages:\n{}",
    content,
    format_messages(messages)
  ));

  let output = generate_object::<EpisodeTitleOutput>(
    vec![
      ChatCompletionRequestMessage::System(system),
      ChatCompletionRequestMessage::User(user),
    ],
    "episodic_title_generation".to_owned(),
    Some("Generate an episodic memory title".to_owned()),
  )
  .await?;

  let title = output.title.trim();
  Ok(if title.is_empty() {
    "Conversation Segment".to_owned()
  } else {
    title.to_owned()
  })
}

async fn try_anchor_episode_lines(lines: &mut [RenderedEpisodeLine]) {
  let candidates = build_time_anchor_candidates(lines);
  if candidates.is_empty() {
    return;
  }

  let output = match request_time_anchor_insertions(&candidates).await {
    Ok(output) => output,
    Err(err) => {
      tracing::warn!(error = %err, "Episode time anchoring failed; using deterministic content");
      return;
    }
  };

  for candidate in &candidates {
    let Some(line) = lines.get_mut(candidate.line_index) else {
      continue;
    };
    let mut insertions: Vec<_> = output
      .insertions
      .iter()
      .filter(|insertion| usize::try_from(insertion.line_index).ok() == Some(candidate.line_index))
      .filter(|insertion| is_valid_time_anchor_insertion(insertion, candidate))
      .cloned()
      .collect();
    insertions.sort_by(|left, right| right.exact_text.len().cmp(&left.exact_text.len()));

    for insertion in insertions {
      let _ = apply_insertion(
        &mut line.content,
        &insertion.exact_text,
        insertion.anchor_text.trim(),
      );
    }
  }
}

async fn request_time_anchor_insertions(
  candidates: &[TimeAnchorCandidateLine],
) -> Result<TimeAnchorOutput, AppError> {
  let system = ChatCompletionRequestSystemMessage::from(EPISODE_TIME_ANCHOR_SYSTEM_PROMPT.trim());
  let user = ChatCompletionRequestUserMessage::from(build_time_anchor_user_content(candidates));

  generate_object::<TimeAnchorOutput>(
    vec![
      ChatCompletionRequestMessage::System(system),
      ChatCompletionRequestMessage::User(user),
    ],
    "episodic_time_anchoring".to_owned(),
    Some("Add grounded time anchors to existing conversation lines".to_owned()),
  )
  .await
}

// ──────────────────────────────────────────────────
// Deterministic Rendering
// ──────────────────────────────────────────────────

fn render_episode_lines(messages: &[Message]) -> Vec<RenderedEpisodeLine> {
  messages
    .iter()
    .enumerate()
    .map(|(line_index, message)| RenderedEpisodeLine {
      line_index,
      timestamp: message.timestamp,
      role: message.role.to_string(),
      content: collapse_inline_whitespace(&message.content),
    })
    .collect()
}

fn render_episode_content(lines: &[RenderedEpisodeLine]) -> String {
  let mut out = String::new();
  let mut current_bucket: Option<(i32, u32, u32, u32)> = None;

  for line in lines {
    let bucket = (
      line.timestamp.year(),
      line.timestamp.month(),
      line.timestamp.day(),
      line.timestamp.hour(),
    );
    if current_bucket != Some(bucket) {
      if !out.is_empty() {
        out.push_str("\n\n");
      }
      let _ = write!(out, "{}", format_at_header(line.timestamp));
      current_bucket = Some(bucket);
      out.push('\n');
    } else {
      out.push('\n');
    }

    let _ = write!(out, "{}: {}", line.role, line.content);
  }

  out.trim_end().to_owned()
}

fn build_time_anchor_candidates(lines: &[RenderedEpisodeLine]) -> Vec<TimeAnchorCandidateLine> {
  lines
    .iter()
    .map(|line| TimeAnchorCandidateLine {
      line_index: line.line_index,
      timestamp: line.timestamp,
      role: line.role.clone(),
      content: line.content.clone(),
    })
    .collect()
}

fn build_time_anchor_user_content(candidates: &[TimeAnchorCandidateLine]) -> String {
  let mut out = String::from(
    "Candidate lines for optional time anchoring.\nUse the provided `spoken_at` timestamp as reference when resolving relative time phrases.\n",
  );

  for candidate in candidates {
    let _ = writeln!(out, "\nline_index={}", candidate.line_index);
    let _ = writeln!(
      out,
      "spoken_at={}",
      candidate.timestamp.format("%Y-%m-%dT%H:%M:%SZ")
    );
    let _ = writeln!(out, "role={}", candidate.role);
    let _ = writeln!(out, "content={}", candidate.content);
  }

  out
}

fn format_messages(messages: &[Message]) -> String {
  messages
    .iter()
    .enumerate()
    .map(|(index, message)| {
      format!(
        "Message {} [{}] {}: {}",
        index + 1,
        message.timestamp.format("%Y-%m-%dT%H:%M:%SZ"),
        message.role,
        message.content
      )
    })
    .collect::<Vec<_>>()
    .join("\n")
}

fn collapse_inline_whitespace(text: &str) -> String {
  text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn month_abbrev(month: u32) -> &'static str {
  match month {
    1 => "Jan",
    2 => "Feb",
    3 => "Mar",
    4 => "Apr",
    5 => "May",
    6 => "Jun",
    7 => "Jul",
    8 => "Aug",
    9 => "Sep",
    10 => "Oct",
    11 => "Nov",
    12 => "Dec",
    _ => "Unknown",
  }
}

fn format_at_header(timestamp: DateTime<Utc>) -> String {
  let hour = timestamp.hour();
  let hour_12 = match hour % 12 {
    0 => 12,
    value => value,
  };
  let meridiem = if hour < 12 { "AM" } else { "PM" };
  format!(
    "Spoken At: {} {}, {} {} {}",
    month_abbrev(timestamp.month()),
    timestamp.day(),
    timestamp.year(),
    hour_12,
    meridiem
  )
}

fn insertion_already_applied(content: &str, exact_text: &str) -> bool {
  content.contains(&format!("{exact_text} ("))
}

fn normalize_anchor_text(text: &str) -> String {
  text
    .split_whitespace()
    .collect::<Vec<_>>()
    .join(" ")
    .to_ascii_lowercase()
}

fn looks_like_grounded_calendar_info(text: &str) -> bool {
  text.chars().any(|c| c.is_ascii_digit())
}

fn apply_insertion(content: &mut String, exact_text: &str, anchor_text: &str) -> bool {
  if insertion_already_applied(content, exact_text) {
    return false;
  }

  let matches: Vec<_> = content.match_indices(exact_text).collect();
  if matches.len() != 1 {
    return false;
  }

  let insert_at = matches[0].0 + exact_text.len();
  content.insert_str(insert_at, &format!(" ({anchor_text})"));
  true
}

fn is_valid_time_anchor_insertion(
  insertion: &TimeAnchorInsertion,
  candidate: &TimeAnchorCandidateLine,
) -> bool {
  // Precision is still part of the LLM schema, but local validation is
  // intentionally structural only.
  let _ = insertion.precision;
  if insertion.exact_text.trim().is_empty() || insertion.anchor_text.trim().is_empty() {
    return false;
  }
  if insertion.anchor_text.contains('(') || insertion.anchor_text.contains(')') {
    return false;
  }
  if !candidate.content.contains(&insertion.exact_text) {
    return false;
  }
  if normalize_anchor_text(&insertion.exact_text) == normalize_anchor_text(&insertion.anchor_text) {
    return false;
  }
  if !looks_like_grounded_calendar_info(&insertion.anchor_text) {
    return false;
  }
  true
}
