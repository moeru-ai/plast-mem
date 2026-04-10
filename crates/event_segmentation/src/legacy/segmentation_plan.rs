use plastmem_shared::Message;
use schemars::JsonSchema;
use serde::Deserialize;

use super::{SurpriseLevel, format_messages};

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SegmentationPlanOutput {
  pub segments: Vec<SegmentationPlanItem>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SegmentationPlanItem {
  pub start_message_index: u32,
  pub surprise_level: SurpriseLevel,
}

#[derive(Debug, Clone)]
pub struct SegmentedConversation {
  pub messages: Vec<Message>,
  pub surprise_level: SurpriseLevel,
}

pub const SEGMENTATION_SYSTEM_PROMPT: &str = r#"
You are segmenting a batch of conversation messages into episodic units.
Return only JSON that matches the schema.

Identify the first message of each new segment.
Start a new segment whenever there is a meaningful topic shift or a clear surprise/discontinuity.
Use HIGH SENSITIVITY to topic shifts.
When boundary placement is uncertain, split.

# Boundary Triggers

1. **Topic change** (highest priority)
 - The conversation moves from one concrete event, question, problem, or activity to another.
 - A previous issue has been answered or wrapped up, and the next messages open a different thread.
 - The new messages are only loosely related to the prior discussion, even if they share the same broad life area.

2. **Intent transition**
 - The purpose of the exchange changes, such as moving from catching up to seeking advice, from one person's update to the other's unrelated update, or from one question to a different question.
 - A new exchange starts after the current one has already reached a natural stopping point.

3. **Temporal markers**
 - Temporal transition phrases such as "earlier", "before", "by the way", "oh right", "also", "anyway", or "speaking of".
 - Any gap over 30 minutes is a strong boundary signal unless the messages are clearly continuing the same unresolved exchange.

4. **Structural signals**
 - Explicit topic-change phrases such as "changing topics", "quick question", or "speaking of which".
 - Closing or wrap-up statements that indicate the current thread is finished.

5. **Surprise & discontinuity**
 - Abrupt emotional reversals or unexpected vulnerability.
 - Sudden shifts between personal/emotional and logistical/factual content.
 - Introduction of a completely new domain.
 - Sharp changes in tone or register.

- **surprise_level:** Measure how abruptly the segment begins relative to the *preceding* segment. The first segment is `low` unless a previous episode is provided as context.
  - `low`: Gradual or routine transition.
  - `high`: Noticeable discontinuity (unexpected emotion, intent reversal, domain change).
  - `extremely_high`: Stark break (shocking event, intense emotion, major domain jump).

# Quality Constraints

- Each item marks the first message of a segment using a 0-based `start_message_index`.
- Return only segment starts. Do not return segment ends.
- Include the first segment start at index 0.
- Indices should be unique and in ascending order.
- If there is no meaningful boundary, return exactly one segment start at 0.
- Prioritize topic independence. Each episode should revolve around one core topic, event, or unresolved exchange.
- A segment should usually stay within 10-15 messages. Longer segments are acceptable only when the messages are still clearly part of the same ongoing topic and splitting would create artificial fragments.
- Do not merge multiple date-separated or topic-separated exchanges into one large "catch-all" segment.
- Focus on choosing the right split points. The system will derive segment ends automatically.
- Return only JSON that matches the schema."#;

pub fn build_segmentation_user_content(
  messages: &[Message],
  prev_episode_content: Option<&str>,
  retry_reason: Option<&str>,
) -> String {
  let formatted = format_messages(messages);
  let request = prev_episode_content.map_or_else(
    || format!("Messages to segment:\n{formatted}"),
    |content| {
      format!(
        "Previous episode content: {content}\n\
         Use this only as reference for the first segment's surprise level.\n\n\
         Messages to segment:\n{formatted}"
      )
    },
  );

  retry_reason.map_or_else(
    || request.clone(),
    |reason| {
      format!(
        "The previous segmentation plan was invalid.\n\
       Failure reason: {reason}\n\n\
       Re-segment the same messages.\n\
       Return only valid 0-based segment start indices.\n\
       Do not include segment ends.\n\
       Do not create catch-all tail segments.\n\n\
       {request}"
      )
    },
  )
}

pub fn resolve_segmentation_plan(
  messages: &[Message],
  items: Vec<SegmentationPlanItem>,
) -> Result<Vec<SegmentedConversation>, String> {
  let batch_len = messages.len();
  if batch_len == 0 {
    return Ok(Vec::new());
  }

  let mut starts = Vec::with_capacity(items.len() + 1);
  for (segment_idx, item) in items.into_iter().enumerate() {
    let start = usize::try_from(item.start_message_index)
      .map_err(|_| format!("segment {segment_idx} start_message_index overflowed usize"))?;

    if start >= batch_len {
      tracing::warn!(
        segment_idx,
        start,
        batch_len,
        "Ignoring out-of-bounds segment start"
      );
      continue;
    }

    starts.push((start, item.surprise_level));
  }

  starts.sort_by_key(|(start, _)| *start);
  starts.dedup_by_key(|(start, _)| *start);

  if starts.first().is_none_or(|(start, _)| *start != 0) {
    tracing::warn!("Segmentation plan omitted start index 0; inserting fallback first segment");
    starts.insert(0, (0, SurpriseLevel::Low));
  }

  let mut resolved = Vec::with_capacity(starts.len());
  for (idx, (start, surprise_level)) in starts.iter().enumerate() {
    let end = starts
      .get(idx + 1)
      .map_or(batch_len - 1, |(next_start, _)| {
        next_start.saturating_sub(1)
      });

    resolved.push(SegmentedConversation {
      messages: messages[*start..=end].to_vec(),
      surprise_level: surprise_level.clone(),
    });
  }

  Ok(resolved)
}

#[cfg(test)]
mod tests {
  use chrono::{TimeZone, Utc};
  use plastmem_shared::MessageRole;

  use super::*;

  fn make_messages(count: usize) -> Vec<Message> {
    (0..count)
      .map(|i| Message {
        role: MessageRole::from("User"),
        content: format!("message {i}"),
        timestamp: Utc.timestamp_opt(i as i64, 0).unwrap(),
      })
      .collect()
  }

  #[test]
  fn resolves_valid_contiguous_plan() {
    let messages = make_messages(5);
    let segments = resolve_segmentation_plan(
      &messages,
      vec![
        SegmentationPlanItem {
          start_message_index: 0,
          surprise_level: SurpriseLevel::Low,
        },
        SegmentationPlanItem {
          start_message_index: 2,
          surprise_level: SurpriseLevel::High,
        },
      ],
    )
    .unwrap();

    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0].messages.len(), 2);
    assert_eq!(segments[1].messages.len(), 3);
  }

  #[test]
  fn inserts_zero_start_when_missing() {
    let messages = make_messages(5);
    let segments = resolve_segmentation_plan(
      &messages,
      vec![SegmentationPlanItem {
        start_message_index: 3,
        surprise_level: SurpriseLevel::Low,
      }],
    )
    .unwrap();

    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0].messages.len(), 3);
    assert_eq!(segments[1].messages.len(), 2);
  }

  #[test]
  fn ignores_out_of_bounds_starts() {
    let messages = make_messages(4);
    let segments = resolve_segmentation_plan(
      &messages,
      vec![
        SegmentationPlanItem {
          start_message_index: 99,
          surprise_level: SurpriseLevel::Low,
        },
        SegmentationPlanItem {
          start_message_index: 2,
          surprise_level: SurpriseLevel::Low,
        },
      ],
    )
    .unwrap();

    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0].messages.len(), 2);
    assert_eq!(segments[1].messages.len(), 2);
  }

  #[test]
  fn deduplicates_and_sorts_starts() {
    let messages = make_messages(4);
    let segments = resolve_segmentation_plan(
      &messages,
      vec![
        SegmentationPlanItem {
          start_message_index: 2,
          surprise_level: SurpriseLevel::Low,
        },
        SegmentationPlanItem {
          start_message_index: 0,
          surprise_level: SurpriseLevel::Low,
        },
        SegmentationPlanItem {
          start_message_index: 2,
          surprise_level: SurpriseLevel::High,
        },
      ],
    )
    .unwrap();

    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0].messages.len(), 2);
    assert_eq!(segments[1].messages.len(), 2);
  }
}
