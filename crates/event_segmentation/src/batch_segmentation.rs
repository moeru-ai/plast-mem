use chrono::TimeDelta;
use plastmem_ai::{
  ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
  ChatCompletionRequestUserMessage, generate_object,
};
use plastmem_core::ConversationMessage;
use plastmem_shared::AppError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryType {
  Soft,
  Hard,
}

#[derive(Debug, Clone)]
pub struct TemporalBoundary {
  pub left_end_seq: i64,
  pub right_start_seq: i64,
  pub gap_minutes: i64,
  pub boundary_type: BoundaryType,
}

#[derive(Debug, Clone)]
pub struct RuleSegOutput {
  pub claim_start_seq: i64,
  pub claim_end_seq: i64,
  pub boundaries: Vec<TemporalBoundary>,
}

#[derive(Debug, Clone)]
pub struct BucketRange {
  pub start_seq: i64,
  pub end_seq: i64,
}

#[derive(Debug, Clone)]
pub struct CandidateSegment {
  pub start_seq: i64,
  pub end_seq: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SegmentClassification {
  LowInfo,
  Informative,
}

#[derive(Debug, Clone)]
pub struct ReviewedSegment {
  pub start_seq: i64,
  pub end_seq: i64,
  pub classification: SegmentClassification,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryOrigin {
  RuleSoft,
  RuleHard,
  LlmLocked,
}

#[derive(Debug, Clone)]
pub struct ReviewedBoundary {
  pub left_end_seq: i64,
  pub right_start_seq: i64,
  pub origin: BoundaryOrigin,
  pub gap_minutes: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PrimitiveClassificationOutput {
  pub classification: SegmentClassification,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PrimitiveSplitOutput {
  pub split_start_message_indices: Vec<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ConstrainedResegmentationOutput {
  pub split_start_message_indices: Vec<u32>,
}

const RULE_SEGMENT_TIME_GAP_THRESHOLD: TimeDelta = TimeDelta::minutes(30);
const HARD_BOUNDARY_TIME_GAP_THRESHOLD: TimeDelta = TimeDelta::hours(3);
const LOW_INFO_LLM_MAX_MESSAGES: i64 = 4;
const PRIMITIVE_SPLIT_TRIGGER_MESSAGES: i64 = 20;
const SECOND_STAGE_RESEG_TRIGGER_MESSAGES: i64 = 30;

const JSON_SCHEMA_REQUIREMENT: &str = r#"
Return only JSON that matches the provided schema.
"#;

const CLASSIFICATION_LABEL_GUIDANCE: &str = r#"
Classification labels:
- `low_info`: acknowledgements, backchannels, thin coordination, utility bookkeeping, short status exchanges, or other low-signal turns with weak long-term retrieval value.
- `informative`: durable facts, plans, decisions, events, constraints, preferences, commitments, or other context likely to matter later.
"#;

const BOUNDARY_TRIGGER_GUIDANCE: &str = r#"
Boundary triggers:
1. Topic change: the conversation moves from one concrete event, question, problem, or activity to another.
   - A previous issue has been answered or wrapped up, and the next messages open a different thread.
   - The new messages are only loosely related to the prior discussion, even if they share the same broad life area.
2. Intent transition: the purpose of the exchange changes after the previous thread has already reached a natural stopping point.
   - Examples: catching up -> advice seeking, one person's update -> the other's unrelated update, one question -> a different question.
3. Temporal markers: temporal transition phrases such as "earlier", "before", "by the way", "oh right", "also", "anyway", or "speaking of".
   - A gap over 30 minutes is a strong boundary signal unless the messages are clearly continuing the same unresolved exchange.
4. Structural signal: explicit pivots, wrap-up statements, or new thread openers.
5. Surprise or discontinuity: abrupt emotional reversals, unexpected vulnerability, major domain jumps, or sharp changes in tone or register.
"#;

const SEGMENT_LENGTH_GUIDANCE: &str = r#"
Segment length guidance:
- A segment should usually stay within 10-15 messages.
- Longer segments are acceptable only when the messages still clearly belong to the same ongoing topic and splitting would create artificial fragments.
- Do not merge multiple topic-separated exchanges into one catch-all segment.
"#;

const INDEX_RULES: &str = r#"
Index rules:
1. Every message is labeled with `[idx=N]`.
2. `start_message_index` and `split_start_message_indices` must use those `idx` values exactly as shown.
3. They are 0-based indices into the provided messages.
4. Do not count lines, timestamps, or tokens yourself.
5. Do not infer indices from external seq values.
"#;

const CANDIDATE_BOUNDARY_HINT_RULES: &str = r#"
Candidate boundaries are hints, not commands:
- keep them if they reflect real internal structure
- remove them if the range should remain one segment
- add new splits only when a clearer internal boundary exists
"#;

const SPLIT_SENSITIVITY_GUIDANCE: &str = r#"
Split sensitivity:
1. Use HIGH SENSITIVITY to topic shifts and meaningful discontinuities.
2. When boundary placement is uncertain, prefer splitting rather than merging unrelated exchanges.
3. Do not use context outside the provided messages.
"#;

fn compose_prompt(sections: &[&str]) -> String {
  sections.join("\n\n")
}

pub fn primitive_classification_system_prompt() -> String {
  compose_prompt(&[
    "You are classifying one conversation segment.",
    JSON_SCHEMA_REQUIREMENT,
    "Return structured JSON with:\n- `classification`",
    CLASSIFICATION_LABEL_GUIDANCE,
    "Rules:\n1. Classify only the provided range.\n2. Do not assume context outside the provided messages.\n3. Use `low_info` only for truly thin exchanges such as greetings, sign-offs, acknowledgements, backchannels, or minimal coordination with weak long-term retrieval value.\n4. If the range contains any durable fact, plan, decision, preference, constraint, event detail, or meaningful social exchange, classify it as `informative`.",
  ])
}

pub fn primitive_split_system_prompt() -> String {
  compose_prompt(&[
    "You are reviewing one long conversation segment.",
    JSON_SCHEMA_REQUIREMENT,
    "Return structured JSON with:\n- `split_start_message_indices`",
    "Your job:\n1. Identify the first message of each later child segment.\n2. Start a new child segment whenever there is a meaningful topic shift or a clear surprise/discontinuity.\n3. Focus only on boundary placement. The system will classify child segments separately.",
    BOUNDARY_TRIGGER_GUIDANCE,
    SEGMENT_LENGTH_GUIDANCE,
    SPLIT_SENSITIVITY_GUIDANCE,
    INDEX_RULES,
    "Output rules:\n1. Return only later split starts. Do not include 0.\n2. Keep indices unique and ascending.\n3. If there is no meaningful boundary, return an empty array.",
  ])
}

pub fn constrained_resegment_system_prompt() -> String {
  compose_prompt(&[
    "You are re-segmenting one informative conversation range.",
    JSON_SCHEMA_REQUIREMENT,
    "Return structured JSON with:\n- `split_start_message_indices`",
    "Your job:\n1. Review the candidate boundaries provided for this range.\n2. Identify the first message of each informative subsegment after boundary review.\n3. Start a new informative subsegment whenever there is a meaningful topic shift or a clear surprise/discontinuity inside the provided range.",
    BOUNDARY_TRIGGER_GUIDANCE,
    SEGMENT_LENGTH_GUIDANCE,
    "Rules:\n1. All content in the provided range should remain informative. Do not re-classify.\n2. Use high sensitivity to real topic shifts. If a candidate boundary is plausibly meaningful, keep the split.\n3. Do not use context outside the provided messages.",
    CANDIDATE_BOUNDARY_HINT_RULES,
    INDEX_RULES,
    "Output rules:\n1. Return only later split starts. Do not include 0.\n2. Keep indices unique and ascending.\n3. If the whole range should remain one informative segment, return an empty array.",
  ])
}

pub fn temporal_rule_segmenter(messages: &[ConversationMessage]) -> Result<RuleSegOutput, String> {
  let first_seq = messages
    .first()
    .ok_or_else(|| "Cannot run temporal rule segmenter on empty message list".to_owned())?
    .seq;
  let last_seq = messages
    .last()
    .ok_or_else(|| "Cannot run temporal rule segmenter on empty message list".to_owned())?
    .seq;

  let mut boundaries = Vec::new();
  for pair in messages.windows(2) {
    let gap = pair[1].timestamp.signed_duration_since(pair[0].timestamp);
    if gap < RULE_SEGMENT_TIME_GAP_THRESHOLD {
      continue;
    }

    boundaries.push(TemporalBoundary {
      left_end_seq: pair[0].seq,
      right_start_seq: pair[1].seq,
      gap_minutes: gap.num_minutes(),
      boundary_type: if gap > HARD_BOUNDARY_TIME_GAP_THRESHOLD {
        BoundaryType::Hard
      } else {
        BoundaryType::Soft
      },
    });
  }

  Ok(RuleSegOutput {
    claim_start_seq: first_seq,
    claim_end_seq: last_seq,
    boundaries,
  })
}

pub fn derive_bucket_ranges(output: &RuleSegOutput) -> Vec<BucketRange> {
  let mut buckets = Vec::with_capacity(output.boundaries.len() + 1);
  let mut start_seq = output.claim_start_seq;

  for boundary in &output.boundaries {
    buckets.push(BucketRange {
      start_seq,
      end_seq: boundary.left_end_seq,
    });
    start_seq = boundary.right_start_seq;
  }

  buckets.push(BucketRange {
    start_seq,
    end_seq: output.claim_end_seq,
  });
  buckets
}

fn validate_segment_message_seqs(
  segment: &CandidateSegment,
  message_seqs: &[i64],
) -> Result<(), String> {
  if message_seqs.is_empty() {
    return Err("Candidate segment has no backing message seqs".to_owned());
  }

  let first_seq = *message_seqs
    .first()
    .ok_or_else(|| "Missing first message seq".to_owned())?;
  let last_seq = *message_seqs
    .last()
    .ok_or_else(|| "Missing last message seq".to_owned())?;
  if first_seq != segment.start_seq || last_seq != segment.end_seq {
    return Err("Segment review messages do not match candidate segment range".to_owned());
  }

  Ok(())
}

fn resolve_split_ranges(
  message_seqs: &[i64],
  split_start_message_indices: Vec<u32>,
) -> Result<Vec<(i64, i64)>, String> {
  let mut split_indices: Vec<usize> = split_start_message_indices
    .into_iter()
    .filter_map(|idx| usize::try_from(idx).ok())
    .filter(|idx| *idx > 0 && *idx < message_seqs.len())
    .collect();
  split_indices.sort_unstable();
  split_indices.dedup();

  let mut starts = Vec::with_capacity(split_indices.len() + 1);
  starts.push(0usize);
  starts.extend(split_indices);

  let mut ranges = Vec::with_capacity(starts.len());
  for (index, start_idx) in starts.iter().enumerate() {
    let end_idx = starts
      .get(index + 1)
      .map_or(message_seqs.len(), |next| *next);
    let start_seq = *message_seqs
      .get(*start_idx)
      .ok_or_else(|| "Resolved empty split segment".to_owned())?;
    let end_seq = *message_seqs
      .get(end_idx.saturating_sub(1))
      .ok_or_else(|| "Resolved empty split segment".to_owned())?;
    ranges.push((start_seq, end_seq));
  }

  Ok(ranges)
}

pub fn resolve_primitive_classification(
  segment: &CandidateSegment,
  message_seqs: &[i64],
  output: PrimitiveClassificationOutput,
) -> Result<Vec<ReviewedSegment>, String> {
  validate_segment_message_seqs(segment, message_seqs)?;
  Ok(vec![ReviewedSegment {
    start_seq: segment.start_seq,
    end_seq: segment.end_seq,
    classification: output.classification,
  }])
}

pub fn resolve_primitive_split(
  segment: &CandidateSegment,
  message_seqs: &[i64],
  output: PrimitiveSplitOutput,
) -> Result<Vec<CandidateSegment>, String> {
  validate_segment_message_seqs(segment, message_seqs)?;
  let ranges = resolve_split_ranges(message_seqs, output.split_start_message_indices)?;

  Ok(
    ranges
      .into_iter()
      .map(|(start_seq, end_seq)| CandidateSegment { start_seq, end_seq })
      .collect(),
  )
}

pub fn resolve_constrained_resegmentation(
  segment: &CandidateSegment,
  message_seqs: &[i64],
  output: ConstrainedResegmentationOutput,
) -> Result<Vec<ReviewedSegment>, String> {
  validate_segment_message_seqs(segment, message_seqs)?;
  let ranges = resolve_split_ranges(message_seqs, output.split_start_message_indices)?;
  Ok(
    ranges
      .into_iter()
      .map(|(start_seq, end_seq)| ReviewedSegment {
        start_seq,
        end_seq,
        classification: SegmentClassification::Informative,
      })
      .collect(),
  )
}

// ──────────────────────────────────────────────────
// Active segmentation flow
// ──────────────────────────────────────────────────

pub async fn primitive_review_llm_segmenter(
  claimed_messages: &[ConversationMessage],
  rule_output: &RuleSegOutput,
) -> Result<(Vec<ReviewedSegment>, Vec<ReviewedBoundary>), AppError> {
  let bucket_ranges = derive_bucket_ranges(rule_output);
  if bucket_ranges.is_empty() {
    return Err(AppError::new(anyhow::anyhow!(
      "Primitive review received empty rule segmentation output"
    )));
  }

  let mut reviewed_segments: Vec<ReviewedSegment> = Vec::new();
  let mut reviewed_boundaries: Vec<ReviewedBoundary> = Vec::new();

  for (bucket_idx, bucket) in bucket_ranges.iter().enumerate() {
    let primitive_candidate = candidate_from_bucket(bucket);
    let primitive_segments =
      classify_or_split_bucket(claimed_messages, &primitive_candidate).await?;
    if primitive_segments.is_empty() {
      return Err(AppError::new(anyhow::anyhow!(
        "Primitive review produced no reviewed segments"
      )));
    }

    if let Some(previous_segment) = reviewed_segments.last() {
      let rule_boundary = rule_output
        .boundaries
        .get(bucket_idx.saturating_sub(1))
        .ok_or_else(|| {
          AppError::new(anyhow::anyhow!(
            "Missing rule boundary between primitive segment groups"
          ))
        })?;
      reviewed_boundaries.push(ReviewedBoundary {
        left_end_seq: previous_segment.end_seq,
        right_start_seq: primitive_segments[0].start_seq,
        origin: map_rule_boundary_origin(rule_boundary.boundary_type),
        gap_minutes: Some(rule_boundary.gap_minutes),
      });
    }

    for pair in primitive_segments.windows(2) {
      reviewed_boundaries.push(ReviewedBoundary {
        left_end_seq: pair[0].end_seq,
        right_start_seq: pair[1].start_seq,
        origin: BoundaryOrigin::LlmLocked,
        gap_minutes: None,
      });
    }

    reviewed_segments.extend(primitive_segments);
  }

  if reviewed_boundaries.len() + 1 != reviewed_segments.len() {
    return Err(AppError::new(anyhow::anyhow!(
      "Primitive review produced misaligned segment and boundary counts"
    )));
  }

  Ok((reviewed_segments, reviewed_boundaries))
}

pub async fn temporal_boundary_review_llm_segmenter(
  claimed_messages: &[ConversationMessage],
  reviewed_segments: &[ReviewedSegment],
  reviewed_boundaries: &[ReviewedBoundary],
) -> Result<Vec<ReviewedSegment>, AppError> {
  if reviewed_segments.is_empty() {
    return Ok(Vec::new());
  }
  if reviewed_boundaries.len() + 1 != reviewed_segments.len() {
    return Err(AppError::new(anyhow::anyhow!(
      "Informative merge sweep received misaligned segment and boundary counts"
    )));
  }

  let mut final_segments = Vec::new();
  let mut index = 0usize;

  while index < reviewed_segments.len() {
    let current = &reviewed_segments[index];
    if current.classification != SegmentClassification::Informative {
      final_segments.push(current.clone());
      index += 1;
      continue;
    }

    let mut group_end = index;
    let mut total_len = segment_message_count(current.start_seq, current.end_seq);

    // Keep merging across RuleSoft boundaries while the group remains below the
    // second-stage threshold. Large or hard-bounded groups stay as-is.
    while total_len < SECOND_STAGE_RESEG_TRIGGER_MESSAGES && group_end + 1 < reviewed_segments.len()
    {
      let boundary = reviewed_boundaries.get(group_end).ok_or_else(|| {
        AppError::new(anyhow::anyhow!(
          "Missing reviewed boundary during informative merge sweep"
        ))
      })?;
      let next_segment = &reviewed_segments[group_end + 1];
      if boundary.origin != BoundaryOrigin::RuleSoft {
        break;
      }
      if next_segment.classification != SegmentClassification::Informative {
        break;
      }

      total_len += segment_message_count(next_segment.start_seq, next_segment.end_seq);
      group_end += 1;
    }

    if group_end > index {
      let merged_segments = constrained_resegment_group(
        claimed_messages,
        &reviewed_segments[index..=group_end],
        &reviewed_boundaries[index..group_end],
      )
      .await?;
      final_segments.extend(merged_segments);
    } else {
      final_segments.push(current.clone());
    }
    index = group_end + 1;
  }

  Ok(final_segments)
}

async fn classify_or_split_bucket(
  claimed_messages: &[ConversationMessage],
  segment: &CandidateSegment,
) -> Result<Vec<ReviewedSegment>, AppError> {
  let segment_messages = slice_segment_messages(claimed_messages, segment)?;
  let segment_len = message_count(segment_messages)?;
  if segment_len <= LOW_INFO_LLM_MAX_MESSAGES {
    let (output, message_seqs) = request_primitive_classification(segment_messages).await?;
    return resolve_primitive_classification(segment, &message_seqs, output)
      .map_err(|reason| AppError::new(anyhow::anyhow!(reason)));
  }

  if segment_len <= PRIMITIVE_SPLIT_TRIGGER_MESSAGES {
    return Ok(vec![reviewed_informative_segment(segment)]);
  }

  let (output, message_seqs) = request_primitive_split(segment_messages).await?;
  match resolve_primitive_split(segment, &message_seqs, output) {
    Ok(child_segments) => {
      let mut reviewed_segments = Vec::with_capacity(child_segments.len());
      for child_segment in child_segments {
        let child_messages = slice_segment_messages(claimed_messages, &child_segment)?;
        if message_count(child_messages)? <= LOW_INFO_LLM_MAX_MESSAGES {
          let (output, message_seqs) = request_primitive_classification(child_messages).await?;
          let child_reviewed =
            resolve_primitive_classification(&child_segment, &message_seqs, output)
              .map_err(|reason| AppError::new(anyhow::anyhow!(reason)))?;
          reviewed_segments.extend(child_reviewed);
        } else {
          reviewed_segments.push(reviewed_informative_segment(&child_segment));
        }
      }
      Ok(reviewed_segments)
    }
    Err(reason) => {
      tracing::warn!(
        start_seq = segment.start_seq,
        end_seq = segment.end_seq,
        error = %reason,
        "Primitive split output invalid; falling back to one informative segment"
      );
      Ok(vec![reviewed_informative_segment(segment)])
    }
  }
}

async fn constrained_resegment_group(
  claimed_messages: &[ConversationMessage],
  group_segments: &[ReviewedSegment],
  group_boundaries: &[ReviewedBoundary],
) -> Result<Vec<ReviewedSegment>, AppError> {
  let merged_segment = CandidateSegment {
    start_seq: group_segments
      .first()
      .ok_or_else(|| AppError::new(anyhow::anyhow!("Missing group start segment")))?
      .start_seq,
    end_seq: group_segments
      .last()
      .ok_or_else(|| AppError::new(anyhow::anyhow!("Missing group end segment")))?
      .end_seq,
  };

  let merged_messages = slice_segment_messages(claimed_messages, &merged_segment)?;
  let (output, message_seqs) =
    request_constrained_resegmentation(merged_messages, group_boundaries).await?;
  resolve_constrained_resegmentation(&merged_segment, &message_seqs, output)
    .map_err(|reason| AppError::new(anyhow::anyhow!(reason)))
}

// ──────────────────────────────────────────────────
// LLM requests
// ──────────────────────────────────────────────────

async fn request_primitive_classification(
  messages: &[ConversationMessage],
) -> Result<(PrimitiveClassificationOutput, Vec<i64>), AppError> {
  let system = ChatCompletionRequestSystemMessage::from(primitive_classification_system_prompt());
  let user = ChatCompletionRequestUserMessage::from(build_plain_segment_user_content(messages));

  let output = generate_object::<PrimitiveClassificationOutput>(
    vec![
      ChatCompletionRequestMessage::System(system),
      ChatCompletionRequestMessage::User(user),
    ],
    "primitive_segment_classification".to_owned(),
    Some("Classify one primitive segment".to_owned()),
  )
  .await?;

  Ok((output, collect_message_seqs(messages)))
}

async fn request_primitive_split(
  messages: &[ConversationMessage],
) -> Result<(PrimitiveSplitOutput, Vec<i64>), AppError> {
  let system = ChatCompletionRequestSystemMessage::from(primitive_split_system_prompt());
  let user = ChatCompletionRequestUserMessage::from(build_plain_segment_user_content(messages));

  let output = generate_object::<PrimitiveSplitOutput>(
    vec![
      ChatCompletionRequestMessage::System(system),
      ChatCompletionRequestMessage::User(user),
    ],
    "primitive_segment_split".to_owned(),
    Some("Split one long primitive segment".to_owned()),
  )
  .await?;

  Ok((output, collect_message_seqs(messages)))
}

async fn request_constrained_resegmentation(
  messages: &[ConversationMessage],
  boundary_hints: &[ReviewedBoundary],
) -> Result<(ConstrainedResegmentationOutput, Vec<i64>), AppError> {
  let system = ChatCompletionRequestSystemMessage::from(constrained_resegment_system_prompt());
  let user = ChatCompletionRequestUserMessage::from(build_constrained_resegment_user_content(
    messages,
    boundary_hints,
  )?);

  let output = generate_object::<ConstrainedResegmentationOutput>(
    vec![
      ChatCompletionRequestMessage::System(system),
      ChatCompletionRequestMessage::User(user),
    ],
    "constrained_resegmentation".to_owned(),
    Some("Re-segment one informative segment group".to_owned()),
  )
  .await?;

  Ok((output, collect_message_seqs(messages)))
}

// ──────────────────────────────────────────────────
// Request content builders
// ──────────────────────────────────────────────────

fn build_plain_segment_user_content(messages: &[ConversationMessage]) -> String {
  let mut output = String::new();
  output.push_str(&format!(
    "Candidate segment:\n- local message count: {}\n- use only the shown `idx` values for any returned start_message_index\n",
    messages.len()
  ));
  for (index, message) in messages.iter().enumerate() {
    output.push_str(&format!(
      "- [idx={}] {} [{}] {}\n",
      index,
      message.timestamp.format("%Y-%m-%dT%H:%M:%SZ"),
      message.role,
      message.content
    ));
  }
  output
}

fn build_constrained_resegment_user_content(
  messages: &[ConversationMessage],
  boundary_hints: &[ReviewedBoundary],
) -> Result<String, AppError> {
  let mut output = String::new();
  output.push_str(&format!(
    "Informative segment group:\n- local message count: {}\n- use only the shown `idx` values for any returned split_start_message_indices\n",
    messages.len()
  ));

  if !boundary_hints.is_empty() {
    output.push_str("\nExisting candidate boundaries inside this range:\n");
    for boundary in boundary_hints {
      let right_start_index = messages
        .iter()
        .position(|message| message.seq == boundary.right_start_seq)
        .ok_or_else(|| {
          AppError::new(anyhow::anyhow!(
            "Boundary hint does not map to any message inside candidate segment"
          ))
        })?;
      let origin = match boundary.origin {
        BoundaryOrigin::RuleSoft => "rule_soft",
        BoundaryOrigin::RuleHard => "rule_hard",
        BoundaryOrigin::LlmLocked => "llm_locked",
      };

      output.push_str(&format!(
        "- split before idx {} (origin={origin})",
        right_start_index
      ));
      if let Some(gap_minutes) = boundary.gap_minutes {
        output.push_str(&format!(", gap_minutes={gap_minutes}"));
      }
      output.push('\n');
    }
    output.push('\n');
  }

  for (index, message) in messages.iter().enumerate() {
    output.push_str(&format!(
      "- [idx={}] {} [{}] {}\n",
      index,
      message.timestamp.format("%Y-%m-%dT%H:%M:%SZ"),
      message.role,
      message.content
    ));
  }

  Ok(output)
}

// ──────────────────────────────────────────────────
// Utilities
// ──────────────────────────────────────────────────

fn map_rule_boundary_origin(boundary_type: BoundaryType) -> BoundaryOrigin {
  match boundary_type {
    BoundaryType::Soft => BoundaryOrigin::RuleSoft,
    BoundaryType::Hard => BoundaryOrigin::RuleHard,
  }
}

fn message_count(messages: &[ConversationMessage]) -> Result<i64, AppError> {
  i64::try_from(messages.len())
    .map_err(|_| AppError::new(anyhow::anyhow!("Segment message count overflow")))
}

fn collect_message_seqs(messages: &[ConversationMessage]) -> Vec<i64> {
  messages.iter().map(|message| message.seq).collect()
}

fn slice_segment_messages<'a>(
  claimed_messages: &'a [ConversationMessage],
  segment: &CandidateSegment,
) -> Result<&'a [ConversationMessage], AppError> {
  let start_idx = claimed_messages
    .iter()
    .position(|message| message.seq == segment.start_seq)
    .ok_or_else(|| {
      AppError::new(anyhow::anyhow!(
        "Candidate segment start_seq does not map to claimed messages"
      ))
    })?;
  let end_idx = claimed_messages[start_idx..]
    .iter()
    .position(|message| message.seq == segment.end_seq)
    .map(|offset| start_idx + offset)
    .ok_or_else(|| {
      AppError::new(anyhow::anyhow!(
        "Candidate segment end_seq does not map to claimed messages"
      ))
    })?;

  Ok(&claimed_messages[start_idx..=end_idx])
}

fn segment_message_count(start_seq: i64, end_seq: i64) -> i64 {
  end_seq - start_seq + 1
}

fn candidate_from_bucket(bucket: &BucketRange) -> CandidateSegment {
  CandidateSegment {
    start_seq: bucket.start_seq,
    end_seq: bucket.end_seq,
  }
}

fn reviewed_informative_segment(segment: &CandidateSegment) -> ReviewedSegment {
  ReviewedSegment {
    start_seq: segment.start_seq,
    end_seq: segment.end_seq,
    classification: SegmentClassification::Informative,
  }
}
