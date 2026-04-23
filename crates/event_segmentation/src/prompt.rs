use plastmem_event::{Event, EventDataToString};

use crate::{EventSegment, EventSegmentReason};

pub fn build_boundary_review_prompt(
  events: &[Event],
  candidates: &[(usize, f32)],
  boundary_budget: usize,
) -> String {
  let mut prompt = format!(
    "Review candidate boundaries for a multilingual dialogue. Fill keep_boundary_indices with at most {} candidate indices that should be kept. Also return decisions for the kept indices, and optionally for nearby rejected candidates when useful. Nearby candidate indices may describe the same transition; choose the single best index, not all of them. Prefer fewer, larger event segments, but keep real pivots between unrelated subjects, activities, plans, stories, intents, or explicit structural pivots. Use topic_shift/topic_intro/intent_shift/activity_shift/structural_cue for true boundaries. Use detail_elaboration/direct_response/closing/noise for continuations, which should usually not be split. Do not split follow-ups, clarifications, examples, greetings, or closing turns. It is valid to keep none.\n\n",
    boundary_budget
  );

  for (candidate_index, candidate_score) in candidates {
    let left = candidate_index.saturating_sub(5);
    let right = (candidate_index + 5).min(events.len());
    prompt.push_str(&format!(
      "Candidate boundary_index={} score={:.3}:\n",
      candidate_index, candidate_score
    ));
    for (offset, event) in events[left..right].iter().enumerate() {
      let index = left + offset;
      if index == *candidate_index {
        prompt.push_str("  <BOUNDARY>\n");
      }
      prompt.push_str(&format!(
        "  [idx={index}] {} {}\n",
        event.timestamp.format("%Y-%m-%dT%H:%M:%SZ"),
        event.data.to_string_without_timestamp()
      ));
    }
    prompt.push('\n');
  }

  prompt
}

pub fn build_short_segment_merge_prompt(
  segments: &[EventSegment],
  short_indices: &[usize],
) -> String {
  let mut prompt = "Review short event segments in a multilingual dialogue. For each listed segment_index, decide whether the current short segment should merge with the immediately previous segment. Merge only when the short segment is a continuation, detail, direct response, greeting/closing, or small conversational tail of the previous event. Keep separate when it starts an independent topic, activity, intent, story, or plan. Use semantic relation across languages; do not rely on English keywords. Return one decision for every listed segment_index.\n\n".to_owned();

  for &index in short_indices {
    prompt.push_str(&format!("segment_index={index}\nprevious_segment_tail:\n"));
    let previous = &segments[index - 1].events;
    let previous_start = previous.len().saturating_sub(8);
    append_events(&mut prompt, &previous[previous_start..]);
    prompt.push_str("current_short_segment:\n");
    if segments[index].reason != EventSegmentReason::InitialSegment {
      prompt.push_str(&format!(
        "  boundary_reason={}\n",
        segments[index].reason.as_ref()
      ));
    }
    append_events(&mut prompt, &segments[index].events);
    prompt.push('\n');
  }

  prompt
}

fn append_events(prompt: &mut String, events: &[Event]) {
  for event in events {
    prompt.push_str(&format!(
      "  {} {}\n",
      event.timestamp.format("%Y-%m-%dT%H:%M:%SZ"),
      event.data.to_string_without_timestamp()
    ));
  }
}

#[cfg(test)]
mod tests {
  use chrono::{TimeZone, Utc};
  use plastmem_event::{Event, EventData, MessageEventData, MessageEventRole};

  use super::{build_boundary_review_prompt, build_short_segment_merge_prompt};
  use crate::{EventSegment, EventSegmentReason};

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
  fn boundary_review_prompt_includes_candidate_marker() {
    let events = vec![
      message_event("a"),
      message_event("b"),
      message_event("c"),
      message_event("d"),
      message_event("e"),
      message_event("f"),
    ];

    let prompt = build_boundary_review_prompt(&events, &[(3, 0.82)], 2);

    assert!(prompt.contains("Candidate boundary_index=3 score=0.820"));
    assert!(prompt.contains("<BOUNDARY>"));
    assert!(prompt.contains("[idx=3]"));
  }

  #[test]
  fn short_segment_merge_prompt_includes_boundary_reason() {
    let segments = vec![
      EventSegment::with_metadata(
        vec![message_event("previous")],
        EventSegmentReason::InitialSegment,
        1.0,
        1.0,
        1.0,
      ),
      EventSegment::with_metadata(
        vec![message_event("current")],
        EventSegmentReason::StructuralCue,
        1.0,
        1.0,
        1.0,
      ),
    ];

    let prompt = build_short_segment_merge_prompt(&segments, &[1]);

    assert!(prompt.contains("segment_index=1"));
    assert!(prompt.contains("boundary_reason=structural_cue"));
  }
}
