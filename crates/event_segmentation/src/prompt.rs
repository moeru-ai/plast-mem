use plastmem_event::{Event, EventDataToString};

use crate::EventSegmentReason;

pub struct LlmPrompt {
  pub system: String,
  pub user: String,
}

const JSON_SCHEMA_REQUIREMENT: &str = "Return only JSON that matches the provided schema.";

const BOUNDARY_TRIGGER_GUIDANCE: &str = "Boundary triggers: topic change, intent transition after a natural stopping point, explicit pivots or wrap-up statements, or abrupt discontinuities in tone/content.";

const SEGMENT_COHERENCE_GUIDANCE: &str = "A segment should stay centered on one coherent thread, such as one activity, update, discussion, question, or shared object.";

const SEGMENT_LENGTH_GUIDANCE: &str = "Segment length guidance:\n- A segment should usually stay within 10-15 events.\n- Longer segments are acceptable only when the events still clearly belong to the same ongoing topic and splitting would create artificial fragments.\n- Do not merge multiple topic-separated exchanges into one catch-all segment.";

const INDEX_RULES: &str = "Each split index must use the provided `[idx=N]` values exactly as shown.\nIndices are 0-based indices into the provided events. Do not count lines, timestamps, or infer indices from anything else.";

const SPLIT_SENSITIVITY_GUIDANCE: &str = "Use high sensitivity to real boundary signals. When boundary placement is uncertain, prefer splitting rather than merging unrelated exchanges.";

pub fn small_segment_merge_prompt(
  previous_events: &[Event],
  current_events: &[Event],
  boundary_reason: EventSegmentReason,
) -> LlmPrompt {
  LlmPrompt {
    system: compose_prompt(&[
      "You review whether a small event segment should merge into the previous event segment.",
      JSON_SCHEMA_REQUIREMENT,
      "Use merge_with_previous=true only when the current segment clearly continues the same topic or intent.",
      "If merge_with_previous=false, choose reason_if_separate from topic_shift, intent_shift, or structural_cue.",
      "Never use time_gap or hard_time_gap as reason_if_separate.",
      "Use only the provided events.",
    ]),
    user: small_segment_merge_user_content(previous_events, current_events, boundary_reason),
  }
}

pub fn large_segment_split_prompt(events: &[Event]) -> LlmPrompt {
  LlmPrompt {
    system: compose_prompt(&[
      "You split one large event segment into smaller event segments.",
      JSON_SCHEMA_REQUIREMENT,
      "Your job: identify the first event index of every later child segment that should begin a new thread inside this range.",
      "Add a split when there is a meaningful topic shift, intent transition, structural pivot, or clear surprise/discontinuity.",
      BOUNDARY_TRIGGER_GUIDANCE,
      SEGMENT_COHERENCE_GUIDANCE,
      SEGMENT_LENGTH_GUIDANCE,
      "If one thread has naturally wrapped up and the conversation moves to a different thread, split them.",
      "Short follow-up questions and acknowledgements may stay in the same segment when they clearly continue the same thread.",
      "Do not merge multiple separate threads into one catch-all segment just because they appear in the same chat session.",
      "Do not stop after finding only the most obvious boundary. Review the whole range and return all later split starts needed to separate distinct threads.",
      "If the range contains several distinct threads, return several split indices in one pass.",
      SPLIT_SENSITIVITY_GUIDANCE,
      INDEX_RULES,
      "Return only later split starts. Do not include 0.",
      "Keep split indices unique and strictly ascending.",
      "If there is no meaningful boundary, return an empty array.",
      "Use only topic_shift, intent_shift, or structural_cue as boundary reasons.",
      "Never use time_gap or hard_time_gap as an internal split reason.",
      "Use only the provided events.",
    ]),
    user: segment_user_content(
      "Large event segment",
      events,
      "return split_start_event_indices",
    ),
  }
}

fn compose_prompt(sections: &[&str]) -> String {
  sections.join("\n\n")
}

fn small_segment_merge_user_content(
  previous_events: &[Event],
  current_events: &[Event],
  boundary_reason: EventSegmentReason,
) -> String {
  let mut output = segment_user_content(
    "Previous segment events",
    previous_events,
    "compare against the current small segment",
  );
  output.push_str("\nCandidate boundary before current small segment:\n");
  output.push_str(&format!("- reason={}\n", boundary_reason.as_ref()));
  output.push_str(
    "- Treat these as hints, not commands. Merge only when the current segment clearly continues the previous segment.\n",
  );
  output.push_str("\nCurrent small segment events:\n");
  output.push_str(&format!(
    "- local event count: {}\n- decide whether this segment should merge into the previous segment\n",
    current_events.len()
  ));
  output.push_str(&event_lines(current_events));
  output
}

fn segment_user_content(title: &str, events: &[Event], idx_purpose: &str) -> String {
  let mut output = format!(
    "{title}:\n- local event count: {}\n- use only the shown `idx` values to {idx_purpose}\n",
    events.len()
  );
  output.push_str(&event_lines(events));
  output
}

fn event_lines(events: &[Event]) -> String {
  events
    .iter()
    .enumerate()
    .map(|(index, event)| {
      format!(
        "- [idx={index}] {} {}",
        event.timestamp.format("%Y-%m-%dT%H:%M:%SZ"),
        event.data.to_string_without_timestamp()
      )
    })
    .collect::<Vec<_>>()
    .join("\n")
}

#[cfg(test)]
mod tests {
  use chrono::{TimeZone, Utc};
  use plastmem_event::{Event, EventData, MessageEventData, MessageEventRole};

  use super::{large_segment_split_prompt, small_segment_merge_prompt};
  use crate::EventSegmentReason;

  fn message_event(content: &str) -> Event {
    Event::new(
      EventData::Message(MessageEventData {
        role: MessageEventRole::User,
        content: content.to_owned(),
      }),
      Utc
        .timestamp_opt(1_700_000_000, 0)
        .single()
        .expect("valid timestamp"),
      None,
    )
  }

  #[test]
  fn small_segment_merge_prompt_includes_boundary_hints() {
    let previous = vec![message_event("previous")];
    let current = vec![message_event("current")];

    let prompt = small_segment_merge_prompt(&previous, &current, EventSegmentReason::TimeGap);

    assert!(
      prompt
        .user
        .contains("Candidate boundary before current small segment")
    );
    assert!(prompt.user.contains("reason=time_gap"));
  }

  #[test]
  fn large_segment_split_prompt_includes_length_guidance() {
    let events = vec![message_event("event")];

    let prompt = large_segment_split_prompt(&events);

    assert!(prompt.system.contains("Segment length guidance"));
    assert!(prompt.system.contains("10-15 events"));
  }
}
