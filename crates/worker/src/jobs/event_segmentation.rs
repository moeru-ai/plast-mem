use std::fmt::Write;

use apalis::prelude::{Data, TaskSink};
use apalis_postgres::PostgresStorage;
use chrono::{DateTime, Datelike, Timelike, Utc};
use fsrs::{DEFAULT_PARAMETERS, FSRS};
use futures::future::{join_all, try_join_all};
use plastmem_ai::{
  ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
  ChatCompletionRequestUserMessage, embed, generate_object,
};
use plastmem_core::{EpisodicMemory, MessageQueue};
use plastmem_entities::episodic_memory;
use plastmem_event::{Event, EventData, MessageEventData, MessageEventRole};
use plastmem_event_segmentation::{EventSegment, EventSegmentReason, EventSegmenter};
use plastmem_shared::{APP_ENV, AppError, Message};
use schemars::JsonSchema;
use sea_orm::{DatabaseConnection, EntityTrait, TransactionTrait};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{MemoryReviewJob, PredictCalibrateJob};

const FLASHBULB_SURPRISE_THRESHOLD: f32 = 0.85;
const FORCE_SINGLE_SEGMENT_QUEUE_LEN: usize = 30;
const DESIRED_RETENTION: f32 = 0.9;
const SURPRISE_BOOST_FACTOR: f32 = 0.5;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventSegmentationJob {
  pub conversation_id: Uuid,
  pub fence_count: i32,
  pub force_process: bool,
  #[serde(default = "default_keep_tail_segment")]
  pub keep_tail_segment: bool,
}

const fn default_keep_tail_segment() -> bool {
  true
}

#[derive(Debug)]
struct BatchSegment {
  messages: Vec<Message>,
  title: String,
  content: String,
  surprise_signal: f32,
}

struct CreatedEpisode {
  id: Uuid,
  surprise: f32,
}

struct PreparedEpisode {
  memory: EpisodicMemory,
  surprise: f32,
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
6. `exact_text` must copy the full original time phrase exactly as it appears in the line.
7. `anchor_text` must add grounded calendar information. Do not repeat, paraphrase, abstract, or generalize the original phrase.
8. The anchor precision must never be more specific than the original phrase supports.
9. Preserve the original phrase in the line and resolve it inline after that phrase.
10. Do not modify non-time text.
11. Do not include parentheses inside `anchor_text`.
";

pub async fn process_event_segmentation(
  job: EventSegmentationJob,
  db: Data<DatabaseConnection>,
  review_storage: Data<PostgresStorage<MemoryReviewJob>>,
  semantic_storage: Data<PostgresStorage<PredictCalibrateJob>>,
) -> Result<(), AppError> {
  let db = &*db;
  let conversation_id = job.conversation_id;
  let fence_count = usize::try_from(job.fence_count).unwrap_or(0);

  let current_messages = MessageQueue::get(conversation_id, db).await?.messages;
  if current_messages.len() < fence_count {
    tracing::debug!(
      conversation_id = %conversation_id,
      fence_count,
      actual = current_messages.len(),
      "Stale event segmentation job; clearing fence"
    );
    MessageQueue::finalize_job(conversation_id, None, db).await?;
    return Ok(());
  }

  let force_due_to_backlog = current_messages.len() >= FORCE_SINGLE_SEGMENT_QUEUE_LEN;
  let should_force_single_segment = job.force_process || force_due_to_backlog;
  let batch_messages = &current_messages[..fence_count];
  let segments = batch_segment(batch_messages).await?;

  if segments.len() == 1 && !should_force_single_segment {
    tracing::info!(conversation_id = %conversation_id, "No split detected; deferring for more messages");
    MessageQueue::clear_fence(conversation_id, db).await?;
    return Ok(());
  }

  let (drain_segments, new_prev_content): (&[BatchSegment], Option<String>) = if segments.len() == 1
  {
    tracing::info!(
      conversation_id = %conversation_id,
      messages = fence_count,
      force_process = job.force_process,
      force_due_to_backlog,
      queue_len = current_messages.len(),
      "Force processing as single episode"
    );
    (&segments[..], None)
  } else if job.keep_tail_segment {
    let to_drain = &segments[..segments.len() - 1];
    let last_content = to_drain.last().map(|segment| segment.content.clone());
    tracing::info!(
      conversation_id = %conversation_id,
      total_segments = segments.len(),
      draining = to_drain.len(),
      keep_tail_segment = job.keep_tail_segment,
      "Event segmentation complete"
    );
    (to_drain, last_content)
  } else {
    tracing::info!(
      conversation_id = %conversation_id,
      total_segments = segments.len(),
      draining = segments.len(),
      keep_tail_segment = job.keep_tail_segment,
      "Event segmentation complete"
    );
    (&segments[..], None)
  };

  let drain_count = drain_segments
    .iter()
    .map(|segment| segment.messages.len())
    .sum::<usize>();

  enqueue_pending_reviews(conversation_id, batch_messages, db, &review_storage).await?;

  let prepared_episodes = prepare_episodes_batch(conversation_id, drain_segments).await?;
  let episodes = persist_episodes_batch(
    conversation_id,
    drain_count,
    new_prev_content,
    &prepared_episodes,
    db,
  )
  .await?;

  enqueue_predict_calibrate_jobs(conversation_id, &episodes, &semantic_storage).await?;

  Ok(())
}

async fn batch_segment(messages: &[Message]) -> Result<Vec<BatchSegment>, AppError> {
  let events = messages.iter().map(message_to_event).collect::<Vec<_>>();
  let event_segments = EventSegmenter::segment(&events).await?;
  let mut segments = event_segments_to_batch_segments(messages, &event_segments)?;

  let generated_entries = try_join_all(
    segments
      .iter()
      .map(|segment| generate_episode_artifacts(&segment.messages)),
  )
  .await?;

  for (segment, (title, content)) in segments.iter_mut().zip(generated_entries) {
    segment.title = title;
    segment.content = content;
  }

  Ok(segments)
}

fn event_segments_to_batch_segments(
  messages: &[Message],
  event_segments: &[EventSegment],
) -> Result<Vec<BatchSegment>, AppError> {
  let mut offset = 0usize;
  let mut segments = Vec::with_capacity(event_segments.len());

  for event_segment in event_segments {
    let len = event_segment.events.len();
    if len == 0 {
      continue;
    }
    if offset + len > messages.len() {
      return Err(AppError::new(anyhow::anyhow!(
        "Event segmenter returned segments longer than source messages"
      )));
    }

    segments.push(BatchSegment {
      messages: messages[offset..offset + len].to_vec(),
      title: String::new(),
      content: String::new(),
      surprise_signal: surprise_signal(event_segment),
    });
    offset += len;
  }

  if offset != messages.len() {
    return Err(AppError::new(anyhow::anyhow!(
      "Event segmenter did not cover all source messages"
    )));
  }

  Ok(segments)
}

fn message_to_event(message: &Message) -> Event {
  let role = match message.role.0.to_ascii_lowercase().as_str() {
    "user" => MessageEventRole::User,
    "assistant" => MessageEventRole::Assistant,
    _ => MessageEventRole::Custom(message.role.0.clone()),
  };

  Event::new(
    EventData::Message(MessageEventData {
      role,
      content: message.content.clone(),
    }),
    message.timestamp,
    None,
  )
}

fn surprise_signal(segment: &EventSegment) -> f32 {
  match segment.reason {
    EventSegmentReason::InitialSegment => 0.2,
    EventSegmentReason::HardTimeGap => 0.9,
    _ => segment
      .score
      .max(segment.boundary_before_confidence)
      .max(segment.boundary_after_confidence)
      .clamp(0.5, 0.8),
  }
}

async fn prepare_episode(
  conversation_id: Uuid,
  messages: &[Message],
  title: &str,
  content: &str,
  surprise_signal: f32,
) -> Result<Option<PreparedEpisode>, AppError> {
  if content.is_empty() {
    tracing::warn!(conversation_id = %conversation_id, "Skipping episode creation: empty content");
    return Ok(None);
  }

  let surprise = surprise_signal.clamp(0.0, 1.0);
  let embedding_input = if title.is_empty() {
    content.to_owned()
  } else {
    format!("{title}. {content}")
  };
  let embedding = embed(&embedding_input).await?;

  let fsrs = FSRS::new(Some(&DEFAULT_PARAMETERS))?;
  let initial_states = fsrs.next_states(None, DESIRED_RETENTION, 0)?;
  let initial_state = initial_states.good.memory;
  let boosted_stability = initial_state.stability * (1.0 + surprise * SURPRISE_BOOST_FACTOR);
  let now = Utc::now();
  let start_at = messages.first().map_or(now, |message| message.timestamp);
  let end_at = messages.last().map_or(now, |message| message.timestamp);

  Ok(Some(PreparedEpisode {
    memory: EpisodicMemory {
      id: Uuid::now_v7(),
      conversation_id,
      messages: messages.to_vec(),
      title: title.to_owned(),
      content: content.to_owned(),
      classification: None,
      embedding,
      stability: boosted_stability,
      difficulty: initial_state.difficulty,
      surprise,
      start_at,
      end_at,
      created_at: now,
      last_reviewed_at: now,
      consolidated_at: None,
    },
    surprise,
  }))
}

async fn prepare_episodes_batch(
  conversation_id: Uuid,
  segments: &[BatchSegment],
) -> Result<Vec<PreparedEpisode>, AppError> {
  let futures = segments.iter().map(|segment| {
    prepare_episode(
      conversation_id,
      &segment.messages,
      &segment.title,
      &segment.content,
      segment.surprise_signal,
    )
  });

  Ok(try_join_all(futures).await?.into_iter().flatten().collect())
}

async fn persist_episodes_batch(
  conversation_id: Uuid,
  drain_count: usize,
  prev_episode_content: Option<String>,
  episodes: &[PreparedEpisode],
  db: &DatabaseConnection,
) -> Result<Vec<CreatedEpisode>, AppError> {
  let txn = db.begin().await?;

  let active_models = episodes
    .iter()
    .map(|episode| {
      let model = episode.memory.to_model()?;
      Ok::<_, AppError>(model.into())
    })
    .collect::<Result<Vec<episodic_memory::ActiveModel>, _>>()?;

  if !active_models.is_empty() {
    episodic_memory::Entity::insert_many(active_models)
      .exec(&txn)
      .await?;
  }

  MessageQueue::drain(conversation_id, drain_count, &txn).await?;
  MessageQueue::finalize_job(conversation_id, prev_episode_content, &txn).await?;
  txn.commit().await?;

  Ok(
    episodes
      .iter()
      .map(|episode| {
        tracing::info!(
          episode_id = %episode.memory.id,
          conversation_id = %conversation_id,
          title = %episode.memory.title,
          messages = episode.memory.messages.len(),
          surprise = episode.surprise,
          "Episode created"
        );

        CreatedEpisode {
          id: episode.memory.id,
          surprise: episode.surprise,
        }
      })
      .collect(),
  )
}

async fn enqueue_pending_reviews(
  conversation_id: Uuid,
  context_messages: &[Message],
  db: &DatabaseConnection,
  review_storage: &PostgresStorage<MemoryReviewJob>,
) -> Result<(), AppError> {
  if !APP_ENV.enable_fsrs_review {
    return Ok(());
  }

  if let Some(pending_reviews) = MessageQueue::take_pending_reviews(conversation_id, db).await? {
    let review_job = MemoryReviewJob {
      pending_reviews,
      context_messages: context_messages.to_vec(),
      reviewed_at: Utc::now(),
    };
    let mut storage = review_storage.clone();
    storage.push(review_job).await?;
  }
  Ok(())
}

async fn enqueue_predict_calibrate_jobs(
  conversation_id: Uuid,
  episodes: &[CreatedEpisode],
  semantic_storage: &PostgresStorage<PredictCalibrateJob>,
) -> Result<(), AppError> {
  if episodes.is_empty() {
    return Ok(());
  }

  let futures = episodes.iter().map(|episode| {
    let mut storage = semantic_storage.clone();
    let job = PredictCalibrateJob {
      conversation_id,
      episode_id: episode.id,
      force: episode.surprise >= FLASHBULB_SURPRISE_THRESHOLD,
    };
    async move { storage.push(job).await }
  });

  let results: Result<Vec<_>, _> = join_all(futures).await.into_iter().collect();
  results?;

  tracing::info!(
    conversation_id = %conversation_id,
    created_jobs = episodes.len(),
    "Enqueued predict-calibrate jobs for new episodes"
  );

  Ok(())
}

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
    let mut insertions = output
      .insertions
      .iter()
      .filter(|insertion| usize::try_from(insertion.line_index).ok() == Some(candidate.line_index))
      .filter(|insertion| is_valid_time_anchor_insertion(insertion, candidate))
      .cloned()
      .collect::<Vec<_>>();
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

  let matches = content.match_indices(exact_text).collect::<Vec<_>>();
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

#[cfg(test)]
mod tests {
  use chrono::TimeZone;
  use plastmem_shared::MessageRole;

  use super::*;

  fn make_messages(count: usize) -> Vec<Message> {
    (0..count)
      .map(|index| Message {
        role: MessageRole::from("user"),
        content: format!("message {index}"),
        timestamp: Utc.timestamp_opt(index as i64, 0).unwrap(),
      })
      .collect()
  }

  #[test]
  fn maps_event_segments_to_contiguous_messages() {
    let messages = make_messages(5);
    let events = messages.iter().map(message_to_event).collect::<Vec<_>>();
    let segments = vec![
      EventSegment::new(events[..2].to_vec(), EventSegmentReason::InitialSegment),
      EventSegment::new(events[2..].to_vec(), EventSegmentReason::TopicShift),
    ];

    let batch_segments = event_segments_to_batch_segments(&messages, &segments).unwrap();

    assert_eq!(batch_segments.len(), 2);
    assert_eq!(batch_segments[0].messages.len(), 2);
    assert_eq!(batch_segments[1].messages.len(), 3);
  }

  #[test]
  fn hard_time_gap_maps_to_high_surprise() {
    let segment = EventSegment::new(Vec::new(), EventSegmentReason::HardTimeGap);
    assert_eq!(surprise_signal(&segment), 0.9);
  }
}
