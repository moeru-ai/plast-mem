use super::*;
use chrono::TimeZone;
use plastmem_shared::MessageRole;

fn record(seq: i64, role: &str, content: &str) -> ConversationMessageRecord {
  ConversationMessageRecord {
    id: Uuid::now_v7(),
    conversation_id: Uuid::nil(),
    seq,
    message: Message {
      role: MessageRole(role.to_owned()),
      content: content.to_owned(),
      timestamp: Utc.timestamp_opt(seq, 0).single().expect("valid timestamp"),
    },
    created_at: Utc.timestamp_opt(seq, 0).single().expect("valid timestamp"),
  }
}

fn record_at(seq: i64, timestamp: i64, role: &str, content: &str) -> ConversationMessageRecord {
  ConversationMessageRecord {
    id: Uuid::now_v7(),
    conversation_id: Uuid::nil(),
    seq,
    message: Message {
      role: MessageRole(role.to_owned()),
      content: content.to_owned(),
      timestamp: Utc
        .timestamp_opt(timestamp, 0)
        .single()
        .expect("valid timestamp"),
    },
    created_at: Utc
      .timestamp_opt(timestamp, 0)
      .single()
      .expect("valid timestamp"),
  }
}

fn message(role: &str, content: &str, timestamp: i64) -> Message {
  Message {
    role: MessageRole(role.to_owned()),
    content: content.to_owned(),
    timestamp: Utc
      .timestamp_opt(timestamp, 0)
      .single()
      .expect("valid timestamp"),
  }
}

fn refined_boundary(
  next_unit_index: usize,
  boundary_reason: BoundaryReason,
  surprise_level: SurpriseLevel,
  candidate_score: f32,
  time_gap: f32,
  semantic_drop: f32,
  online_surprise_prior: f32,
) -> RefinedBoundary {
  RefinedBoundary {
    next_unit_index,
    boundary_reason,
    surprise_level,
    candidate_score,
    time_gap,
    semantic_drop,
    online_surprise_prior,
    micro_exchange_penalty: 0.0,
  }
}

#[test]
fn candidate_scorer_skips_plain_question_answer_exchange() {
  let units = build_analysis_units(&[
    record(0, "John", "What game are you playing right now?"),
    record(1, "James", "I'm playing The Witcher 3 at the moment."),
    record(2, "John", "Nice, I keep hearing good things about it."),
  ]);

  let candidates = score_candidate_boundaries(&units, None);
  assert!(candidates.is_empty());
}

#[test]
fn candidate_scorer_keeps_temporal_gap_boundary() {
  let units = build_analysis_units(&[
    record_at(0, 0, "John", "What game are you playing right now?"),
    record_at(1, 60, "James", "I'm playing The Witcher 3 at the moment."),
    record_at(2, 60 * 60 * 4, "John", "How was your trip last weekend?"),
  ]);

  let candidates = score_candidate_boundaries(&units, None);
  assert!(
    candidates
      .iter()
      .any(|candidate| candidate.next_unit_index == 2)
  );
}

#[test]
fn candidate_scorer_keeps_clear_same_session_topic_shift() {
  let units = build_analysis_units(&[
    record(
      0,
      "John",
      "I've been playing The Witcher 3 a lot this week.",
    ),
    record(
      1,
      "James",
      "Nice, I love open world games and long story quests.",
    ),
    record(
      2,
      "John",
      "My two dogs kept me busy at the park this morning.",
    ),
    record(
      3,
      "James",
      "Dogs always make the day better, especially energetic ones.",
    ),
  ]);

  let candidates = score_candidate_boundaries(&units, None);
  assert!(
    candidates
      .iter()
      .any(|candidate| candidate.next_unit_index == 2)
  );
}

#[test]
fn candidate_scorer_suppresses_dense_same_session_cluster() {
  let units = build_analysis_units(&[
    record(
      12,
      "John",
      "Aww, they're adorable! What are the names of your pets? And what are your plans for the app?",
    ),
    record(
      13,
      "James",
      "Max and Daisy. The goal is to connect pet owners with reliable dog walkers and provide helpful pet care guidance.",
    ),
    record(
      14,
      "John",
      "Sounds good, James! What sets it apart from other existing apps?",
    ),
    record(
      15,
      "James",
      "The personal touch really sets it apart. Users can add their pup's preferences and needs.",
    ),
    record(
      16,
      "John",
      "That's a great idea! What motivates you to work on your programming projects?",
    ),
    record(
      17,
      "James",
      "Creating something and seeing it come to life gives me a great sense of accomplishment.",
    ),
    record(
      18,
      "John",
      "What are you working on that has you feeling so accomplished?",
    ),
    record(
      19,
      "James",
      "I'm working on a game project I've wanted to make since I was a kid.",
    ),
  ]);

  let candidates = score_candidate_boundaries(&units, None);
  let dense_candidates = candidates
    .iter()
    .filter(|candidate| candidate.time_gap == 0.0 && candidate.cue_phrase == 0.0)
    .count();
  assert!(dense_candidates <= 1, "dense candidates: {candidates:?}");
}

#[test]
fn stabilize_refined_boundaries_rejects_tiny_non_temporal_splits() {
  let units = build_analysis_units(&[
    record(
      0,
      "John",
      "I've been planning a pet care app for dog owners.",
    ),
    record(
      1,
      "James",
      "That app sounds useful for people who need reliable dog walkers.",
    ),
    record(
      2,
      "John",
      "I also want the app to track feeding and vet reminders.",
    ),
    record(
      3,
      "James",
      "That would make it even more useful for busy pet parents.",
    ),
    record(
      4,
      "John",
      "Yesterday I went bowling after work and got two strikes.",
    ),
    record(
      5,
      "James",
      "Bowling always feels great when the shots line up.",
    ),
    record(
      6,
      "John",
      "After bowling I came home and kept iterating on the pet app.",
    ),
  ]);
  let boundaries = vec![
    refined_boundary(
      2,
      BoundaryReason::TopicShift,
      SurpriseLevel::High,
      1.7,
      0.0,
      0.84,
      0.25,
    ),
    refined_boundary(
      4,
      BoundaryReason::TopicShift,
      SurpriseLevel::High,
      1.8,
      0.0,
      0.82,
      0.22,
    ),
    refined_boundary(
      6,
      BoundaryReason::TopicShift,
      SurpriseLevel::High,
      2.15,
      0.0,
      0.9,
      0.3,
    ),
  ];

  let stabilized = stabilize_refined_boundaries(&units, &boundaries, true);
  assert_eq!(stabilized.len(), 1);
  assert_eq!(stabilized[0].next_unit_index, 4);
}

#[test]
fn stabilize_refined_boundaries_keeps_temporal_gap_even_when_short() {
  let units = build_analysis_units(&[
    record_at(0, 0, "John", "I played games last night."),
    record_at(1, 60, "James", "Nice, what did you play?"),
    record_at(
      2,
      60 * 60 * 4,
      "John",
      "By the way, I adopted a new dog this afternoon.",
    ),
  ]);
  let boundaries = vec![refined_boundary(
    2,
    BoundaryReason::TemporalGap,
    SurpriseLevel::High,
    1.65,
    0.8,
    0.5,
    0.3,
  )];

  let stabilized = stabilize_refined_boundaries(&units, &boundaries, false);
  assert_eq!(stabilized.len(), 1);
  assert_eq!(stabilized[0].next_unit_index, 2);
}

#[test]
fn stabilize_refined_boundaries_allows_strong_topic_shift_after_large_same_session_segment() {
  let mut records = Vec::new();
  for seq in 0..18 {
    let role = if seq % 2 == 0 { "John" } else { "James" };
    records.push(record(
      seq,
      role,
      "We kept talking about a shared game project, coding details, and gameplay ideas.",
    ));
  }
  records.push(record(
    18,
    "John",
    "I spent last weekend hiking with my dogs near the lake.",
  ));
  records.push(record(
    19,
    "James",
    "The hike sounds great, and the dogs must have loved the trails.",
  ));
  records.push(record(
    20,
    "John",
    "They absolutely loved the water and the long nature walk.",
  ));
  records.push(record(
    21,
    "James",
    "Nature days with dogs always feel refreshing and memorable.",
  ));

  let units = build_analysis_units(&records);
  let boundaries = vec![refined_boundary(
    18,
    BoundaryReason::TopicShift,
    SurpriseLevel::High,
    1.8,
    0.0,
    0.82,
    0.2,
  )];

  let stabilized = stabilize_refined_boundaries(&units, &boundaries, true);
  assert_eq!(stabilized.len(), 1);
  assert_eq!(stabilized[0].next_unit_index, 18);
}

#[test]
fn merge_refined_boundaries_keeps_planner_only_boundary() {
  let planned = refined_boundary(
    6,
    BoundaryReason::TopicShift,
    SurpriseLevel::High,
    2.0,
    0.0,
    0.82,
    0.3,
  );

  let merged = merge_refined_boundaries(Vec::new(), vec![planned]);

  assert_eq!(merged.len(), 1);
  assert_eq!(merged[0].next_unit_index, 6);
  assert_eq!(merged[0].boundary_reason, BoundaryReason::TopicShift);
}

#[test]
fn normalize_closed_spans_merges_weak_singleton_into_more_affine_neighbor() {
  let spans = vec![
    ClosedSpan {
      start_seq: 0,
      end_seq: 1,
      messages: vec![
        message("John", "My dogs had a checkup at the vet today.", 0),
        message("James", "I hope Max and Daisy are doing well.", 1),
      ],
      boundary_reason: BoundaryReason::SessionBreak,
      surprise_level: SurpriseLevel::Low,
    },
    ClosedSpan {
      start_seq: 2,
      end_seq: 2,
      messages: vec![message(
        "John",
        "The vet said Daisy needs more exercise.",
        2,
      )],
      boundary_reason: BoundaryReason::TopicShift,
      surprise_level: SurpriseLevel::Low,
    },
    ClosedSpan {
      start_seq: 3,
      end_seq: 4,
      messages: vec![
        message("James", "I went bowling after work yesterday.", 3),
        message("John", "Bowling sounds fun for a weekend outing.", 4),
      ],
      boundary_reason: BoundaryReason::TopicShift,
      surprise_level: SurpriseLevel::Low,
    },
  ];

  let normalized = normalize_closed_spans(spans);
  assert_eq!(normalized.len(), 2);
  assert_eq!(normalized[0].start_seq, 0);
  assert_eq!(normalized[0].end_seq, 2);
  assert_eq!(normalized[1].start_seq, 3);
}

#[test]
fn normalize_closed_spans_can_merge_short_high_span_when_affinity_is_clear() {
  let spans = vec![
    ClosedSpan {
      start_seq: 0,
      end_seq: 1,
      messages: vec![
        message("John", "My dogs had a checkup at the vet today.", 0),
        message(
          "James",
          "The vet said Daisy is healthy and just needs more exercise.",
          1,
        ),
      ],
      boundary_reason: BoundaryReason::SessionBreak,
      surprise_level: SurpriseLevel::Low,
    },
    ClosedSpan {
      start_seq: 2,
      end_seq: 3,
      messages: vec![
        message("John", "Daisy needs more exercise after the vet visit.", 2),
        message(
          "James",
          "I should probably walk Daisy more after dinner.",
          3,
        ),
      ],
      boundary_reason: BoundaryReason::TopicShift,
      surprise_level: SurpriseLevel::High,
    },
    ClosedSpan {
      start_seq: 4,
      end_seq: 5,
      messages: vec![
        message("John", "I went bowling after work yesterday.", 4),
        message("James", "Bowling sounds fun for a weekend outing.", 5),
      ],
      boundary_reason: BoundaryReason::TopicShift,
      surprise_level: SurpriseLevel::Low,
    },
  ];

  let normalized = normalize_closed_spans(spans);
  assert_eq!(normalized.len(), 2);
  assert_eq!(normalized[0].start_seq, 0);
  assert_eq!(normalized[0].end_seq, 3);
  assert_eq!(normalized[1].start_seq, 4);
}

#[test]
fn normalize_closed_spans_absorbs_trailing_singleton_before_temporal_gap() {
  let spans = vec![
    ClosedSpan {
      start_seq: 58,
      end_seq: 79,
      messages: vec![
        message(
          "John",
          "I finally advanced to the next level in the game.",
          58,
        ),
        message(
          "James",
          "That sounds like a huge accomplishment after all the effort.",
          59,
        ),
        message("John", "Trying new genres has been exciting lately.", 60),
      ],
      boundary_reason: BoundaryReason::TemporalGap,
      surprise_level: SurpriseLevel::High,
    },
    ClosedSpan {
      start_seq: 80,
      end_seq: 80,
      messages: vec![message(
        "John",
        "Thanks! Can't wait to hear about it. Bye!",
        61,
      )],
      boundary_reason: BoundaryReason::SessionBreak,
      surprise_level: SurpriseLevel::Low,
    },
    ClosedSpan {
      start_seq: 81,
      end_seq: 90,
      messages: vec![
        message("John", "Hey James! Long time no chat.", 1_000_000),
        message(
          "James",
          "I joined an online gaming tournament yesterday.",
          1_000_060,
        ),
      ],
      boundary_reason: BoundaryReason::TemporalGap,
      surprise_level: SurpriseLevel::High,
    },
  ];

  let normalized = normalize_closed_spans(spans);
  assert_eq!(normalized.len(), 2);
  assert_eq!(normalized[0].start_seq, 58);
  assert_eq!(normalized[0].end_seq, 80);
  assert_eq!(normalized[1].start_seq, 81);
}
