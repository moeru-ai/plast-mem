use plastmem_event::{Event, EventDataToString};

pub struct LlmPrompt {
  pub system: String,
  pub user: String,
}

const JSON_SCHEMA_REQUIREMENT: &str = "Return only JSON that matches the provided schema.";

const BOUNDARY_TRIGGER_GUIDANCE: &str = "Boundary triggers: topic change, intent transition after a natural stopping point, explicit pivots or wrap-up statements, or abrupt discontinuities in tone/content.";

const SEGMENT_COHERENCE_GUIDANCE: &str = "A segment should stay centered on one coherent thread, such as one activity, update, discussion, question, or shared object.";

const INDEX_RULES: &str = "Each split index must use the provided `[idx=N]` values exactly as shown.\nIndices are 0-based indices into the provided events. Do not count lines, timestamps, or infer indices from anything else.";

const SPLIT_SENSITIVITY_GUIDANCE: &str = "Use high sensitivity to real boundary signals. When boundary placement is uncertain, prefer splitting rather than merging unrelated exchanges.";

pub fn small_segment_merge_prompt(
  previous_events: &[Event],
  current_events: &[Event],
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
    user: small_segment_merge_user_content(previous_events, current_events),
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

fn small_segment_merge_user_content(previous_events: &[Event], current_events: &[Event]) -> String {
  let mut output = segment_user_content(
    "Previous segment events",
    previous_events,
    "compare against the current small segment",
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
