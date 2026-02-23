use plastmem_ai::{
  ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
  ChatCompletionRequestUserMessage, generate_object,
};
use plastmem_shared::{AppError, Message};
use schemars::JsonSchema;
use serde::Deserialize;

// ──────────────────────────────────────────────────
// Public types
// ──────────────────────────────────────────────────

/// Surprise level of a segment relative to the preceding segment.
/// Maps to a numeric signal used for FSRS stability boosting.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SurpriseLevel {
  /// Routine topic transition or gradual shift — no notable discontinuity.
  Low,
  /// Noticeable discontinuity: emotional shift, intent reversal, or unexpected turn.
  High,
  /// Stark break: shocking event, intense emotion, or abrupt major domain jump (flashbulb).
  ExtremelyHigh,
}

impl SurpriseLevel {
  /// Convert to a numeric surprise signal for FSRS stability boost.
  /// `extremely_high` (0.9) exceeds `FLASHBULB_SURPRISE_THRESHOLD` (0.85).
  pub fn to_signal(&self) -> f32 {
    match self {
      SurpriseLevel::Low => 0.2,
      SurpriseLevel::High => 0.6,
      SurpriseLevel::ExtremelyHigh => 0.9,
    }
  }
}

/// A resolved segment after batch LLM segmentation.
pub struct BatchSegment {
  /// The messages belonging to this segment (resolved via sequential slicing).
  pub messages: Vec<Message>,
  /// Concise episode title (5–15 words).
  pub title: String,
  /// Third-person narrative summary (≤50 words).
  pub summary: String,
  /// How abruptly this segment began relative to the preceding segment.
  pub surprise_level: SurpriseLevel,
}

// ──────────────────────────────────────────────────
// LLM structured output schema
// ──────────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
struct SegmentationOutput {
  segments: Vec<SegmentItem>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SegmentItem {
  /// 0-indexed position of the first message in this segment.
  start_message_index: u32,
  /// 0-indexed position of the last message in this segment (inclusive).
  end_message_index: u32,
  /// Number of messages in this segment. Must equal end_message_index - start_message_index + 1.
  /// This is the authoritative field used for sequential slicing.
  num_messages: u32,
  /// Concise title (5–15 words) capturing the episode's core theme.
  title: String,
  /// Third-person narrative summary, ≤50 words.
  summary: String,
  /// How abruptly this segment begins relative to the preceding one.
  surprise_level: SurpriseLevel,
}

// ──────────────────────────────────────────────────
// Prompt
// ──────────────────────────────────────────────────

const SEGMENTATION_SYSTEM_PROMPT: &str = "\
# Instruction

You are an episodic memory segmentation system (Event Segmentation Theory).
Segment the conversation into episodes — moments where the current event model no longer predicts the next turn.

A segment boundary MUST be created when **either**:
- The topic or intent changes meaningfully, OR
- The new message is surprising or discontinuous relative to prior context.

When in doubt, split rather than merge.

---

# Output Format

Return a JSON list of segments. Each segment must include:
- `start_message_index` — 0-indexed position of the first message (inclusive)
- `end_message_index` — 0-indexed position of the last message (inclusive)
- `num_messages` — number of messages; must equal `end_message_index − start_message_index + 1`
- `title` — 5–15 words capturing the core theme
- `summary` — ≤50 words, third-person narrative (e.g., \"The user asked X; the assistant explained Y…\")
- `surprise_level` — `low` | `high` | `extremely_high` (relative to the preceding segment)

---

# Segmentation Rules

## 1. Topic-Aware Rules
- Group consecutive messages sharing the same semantic focus, goal, or activity.
- A boundary occurs when subject matter, intent, or activity changes meaningfully.
- Subtopic changes count (e.g., emotional support → career advice → casual chat).
- Watch for discourse markers: \"by the way\", \"anyway\", \"换个话题\", \"对了\" — these signal deliberate transitions.
- Intent shifts count: chatting→deciding, venting→requesting help.

## 2. Surprise-Aware Rules
Create a boundary if a message diverges abruptly from prior context:
- Sudden emotional reversal or unexpected vulnerability
- Shift between personal/emotional and logistical/factual content
- Introduction of a new domain (health, work, relationships, finance, etc.)
- Sharp change in tone, register, or a notable time gap (visible in timestamps)

## 3. Fusion Policy
- A boundary is created if **either** channel triggers — topic shift OR surprise.
- Prefer finer granularity: when in doubt, split rather than merge.

## 4. Surprise Level
Measures how abruptly this segment begins relative to the preceding segment:
- `low` — gradual or routine transition
- `high` — noticeable discontinuity: unexpected emotion, intent reversal, or domain change
- `extremely_high` — stark break: shocking event, intense emotion, or major domain jump

First segment: assess relative to the previous episode summary if provided; otherwise use `low`.

---

# Quality Requirements
- Segments must be consecutive, non-overlapping, and cover all messages exactly once.
- The first segment must start at message index 0.
- `num_messages` is the authoritative field for slicing and must be accurate.
- All `num_messages` values must sum to the total input message count.
- A single coherent conversation must return exactly one segment covering all messages.";

// ──────────────────────────────────────────────────
// Batch segmentation
// ──────────────────────────────────────────────────

/// Format messages for LLM input: `[idx] timestamp [role] content`
fn format_messages(messages: &[Message]) -> String {
  messages
    .iter()
    .enumerate()
    .map(|(i, m)| {
      format!(
        "[{}] {} [{}] {}",
        i,
        m.timestamp.format("%Y-%m-%dT%H:%M:%SZ"),
        m.role,
        m.content
      )
    })
    .collect::<Vec<_>>()
    .join("\n")
}

/// Segment a batch of messages into episodes using a single LLM call.
///
/// `prev_episode_summary` is the summary of the last drained episode from the previous batch,
/// used as the reference point for the first segment's `surprise_level`.
///
/// Returns segments in order. The last segment is NOT drained (caller handles drain).
pub async fn batch_segment(
  messages: &[Message],
  prev_episode_summary: Option<&str>,
) -> Result<Vec<BatchSegment>, AppError> {
  let formatted = format_messages(messages);

  let user_content = match prev_episode_summary {
    Some(summary) => format!(
      "Previous episode: {summary}\n\
       Use this as the reference point for the first segment's surprise_level.\n\n\
       Messages to segment:\n{formatted}"
    ),
    None => format!("Messages to segment:\n{formatted}"),
  };

  let system = ChatCompletionRequestSystemMessage::from(SEGMENTATION_SYSTEM_PROMPT);
  let user = ChatCompletionRequestUserMessage::from(user_content);

  let output = generate_object::<SegmentationOutput>(
    vec![
      ChatCompletionRequestMessage::System(system),
      ChatCompletionRequestMessage::User(user),
    ],
    "batch_segmentation".to_owned(),
    Some("Batch episodic memory segmentation".to_owned()),
  )
  .await?;

  // Resolve segments using sequential slicing (num_messages is authoritative).
  // This makes the output robust to LLM inconsistencies in start/end indices.
  let batch_len = messages.len();
  let mut resolved = Vec::with_capacity(output.segments.len());
  let mut processed_up_to: usize = 0;

  for (i, item) in output.segments.into_iter().enumerate() {
    let start = processed_up_to;
    let count = item.num_messages as usize;
    let end = (start + count).min(batch_len);

    // Ensure we don't create empty segments
    if start >= batch_len {
      tracing::warn!(
        segment_idx = i,
        batch_len,
        start,
        "LLM segment out of bounds, skipping"
      );
      break;
    }

    processed_up_to = end;

    resolved.push(BatchSegment {
      messages: messages[start..end].to_vec(),
      title: item.title,
      summary: item.summary,
      surprise_level: item.surprise_level,
    });
  }

  // If LLM under-counted, absorb remaining messages into the last segment
  if processed_up_to < batch_len {
    if let Some(last) = resolved.last_mut() {
      last
        .messages
        .extend_from_slice(&messages[processed_up_to..]);
      tracing::warn!(
        remaining = batch_len - processed_up_to,
        "LLM under-counted messages; absorbed into last segment"
      );
    }
  }

  // Fallback: if LLM returned empty segments, treat everything as one segment
  if resolved.is_empty() {
    tracing::warn!("LLM returned empty segments; treating entire batch as one segment");
    resolved.push(BatchSegment {
      messages: messages.to_vec(),
      title: String::new(),
      summary: String::new(),
      surprise_level: SurpriseLevel::Low,
    });
  }

  Ok(resolved)
}
