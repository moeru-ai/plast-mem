use async_openai::types::{
  ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
  ChatCompletionRequestUserMessage,
};
use plastmem_shared::{AppError, Message};
use schemars::JsonSchema;
use serde::Deserialize;

mod embed;
pub use embed::embed;

mod generate_object;
pub use generate_object::generate_object;

mod generate_text;
pub use generate_text::generate_text;

fn format_messages(messages: &[Message]) -> String {
  messages
    .iter()
    .map(|m| format!("{}: {}", m.role, m.content))
    .collect::<Vec<_>>()
    .join("\n")
}

/// Structured output from event segmentation LLM call.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct EventSegmentationOutput {
  /// "create" if the conversation contains significant content, "skip" if trivial
  pub action: String,
  /// Concise summary of the conversation (only when action = "create")
  pub summary: Option<String>,
  /// Prediction error / surprise score (0.0 ~ 1.0)
  /// 0 = fully expected, 1 = complete surprise
  pub surprise: f32,
  /// Why segmentation occurred: "ContentShift", "GoalCompletion", or "PredictionError"
  pub boundary_type: String,
}

const SEGMENT_SYSTEM_CHECK: &str = "\
You are an event segmentation analyzer. Analyze the conversation and produce a structured assessment.

1. **action**: Decide if the conversation contains significant content worth remembering.
   - \"create\" if meaningful (important information, events, decisions, or context)
   - \"skip\" if trivial (greetings, small talk, or unimportant exchanges)
   Be selective - only \"create\" if there's substantive content.

2. **summary**: If action is \"create\", provide a clear and concise summary. If \"skip\", set to null.

3. **surprise**: Rate the prediction error on a 0.0 to 1.0 scale:
   - 0.0 = fully expected, no new information
   - 0.3 = minor information gain
   - 0.7 = significant pivot or revelation
   - 1.0 = complete surprise, model-breaking

4. **boundary_type**: Categorize why this segment is distinct:
   - \"ContentShift\" = topic or subject matter changed
   - \"GoalCompletion\" = task, goal, or intention completed
   - \"PredictionError\" = unexpected event, surprise > 0.7";

const SEGMENT_SYSTEM_FORCE: &str = "\
You are an event segmentation analyzer. This conversation segment must be summarized (no skipping). Produce a structured assessment.

1. **action**: Always \"create\".

2. **summary**: Provide a clear and concise summary of the conversation.

3. **surprise**: Rate the prediction error on a 0.0 to 1.0 scale:
   - 0.0 = fully expected, no new information
   - 0.3 = minor information gain
   - 0.7 = significant pivot or revelation
   - 1.0 = complete surprise, model-breaking

4. **boundary_type**: Categorize why this segment is distinct:
   - \"ContentShift\" = topic or subject matter changed
   - \"GoalCompletion\" = task, goal, or intention completed
   - \"PredictionError\" = unexpected event, surprise > 0.7";

/// Analyzes messages for event segmentation using structured output.
///
/// When `check` is true, the LLM may return action="skip" for trivial content.
/// When `check` is false, the LLM always creates a summary.
///
/// Returns surprise score and boundary type alongside the action/summary.
pub async fn segment_events(
  messages: &[Message],
  check: bool,
) -> Result<EventSegmentationOutput, AppError> {
  let system_prompt = if check {
    SEGMENT_SYSTEM_CHECK
  } else {
    SEGMENT_SYSTEM_FORCE
  };

  let system = ChatCompletionRequestSystemMessage::from(system_prompt);
  let user = ChatCompletionRequestUserMessage::from(format_messages(messages));

  generate_object::<EventSegmentationOutput>(
    vec![
      ChatCompletionRequestMessage::System(system),
      ChatCompletionRequestMessage::User(user),
    ],
    "event_segmentation".to_owned(),
    Some("Event segmentation analysis with surprise and boundary type".to_owned()),
  )
  .await
}
