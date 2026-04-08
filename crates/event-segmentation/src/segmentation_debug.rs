use std::{
  env, fs,
  io::{self, Read},
  process::ExitCode,
};

use plastmem_shared::Message;
use plastmem_event_segmentation::{
  DebugSegmentationMode, DebugSegmentationTrace, debug_segment_messages,
  debug_segment_messages_with_trace,
};

struct Args {
  detail: bool,
  input_path: Option<String>,
  json: bool,
  mode: DebugSegmentationMode,
  trace: bool,
}

fn parse_args() -> Result<Args, String> {
  let mut detail = false;
  let mut input_path = None;
  let mut json = false;
  let mut mode = DebugSegmentationMode::FullLlm;
  let mut trace = false;

  for arg in env::args().skip(1) {
    match arg.as_str() {
      "--help" | "-h" => return Err(usage()),
      "--detail" => detail = true,
      "--json" => json = true,
      "--trace" => trace = true,
      "--no-llm" => mode = DebugSegmentationMode::Deterministic,
      "--planner-only" => mode = DebugSegmentationMode::PlannerOnly,
      "--embedding-planner" => mode = DebugSegmentationMode::EmbeddingPlanner,
      "-" => input_path = None,
      value if value.starts_with('-') => {
        return Err(format!("unknown option: {value}\n\n{}", usage()));
      }
      value => {
        if input_path.replace(value.to_owned()).is_some() {
          return Err(format!("multiple input files provided\n\n{}", usage()));
        }
      }
    }
  }

  Ok(Args {
    detail,
    input_path,
    json,
    mode,
    trace,
  })
}

fn usage() -> String {
  "Usage: cargo run -p plastmem_event_segmentation --features segmentation-debug --bin segmentation_debug -- [--no-llm|--planner-only|--embedding-planner] [--json|--detail|--trace] [messages.json|-]\n\n\
   Input is a JSON array of messages:\n\
   [{\"role\":\"John\",\"content\":\"...\",\"timestamp\":\"2022-03-16T12:00:00Z\"}]"
    .to_owned()
}

fn read_input(path: Option<&str>) -> Result<String, String> {
  match path {
    Some(path) => {
      fs::read_to_string(path).map_err(|error| format!("failed to read {path}: {error}"))
    }
    None => {
      let mut input = String::new();
      io::stdin()
        .read_to_string(&mut input)
        .map_err(|error| format!("failed to read stdin: {error}"))?;
      Ok(input)
    }
  }
}

fn median(sorted: &[usize]) -> f32 {
  if sorted.is_empty() {
    return 0.0;
  }

  let mid = sorted.len() / 2;
  if sorted.len() % 2 == 0 {
    (sorted[mid - 1] + sorted[mid]) as f32 / 2.0
  } else {
    sorted[mid] as f32
  }
}

fn percentile(sorted: &[usize], percentile: f32) -> usize {
  if sorted.is_empty() {
    return 0;
  }

  let index = ((sorted.len() - 1) as f32 * percentile).round() as usize;
  sorted[index.min(sorted.len() - 1)]
}

fn print_summary(trace: &DebugSegmentationTrace, mode: DebugSegmentationMode, detail: bool) {
  let spans = &trace.spans;
  let total_messages = spans.iter().map(|span| span.message_count).sum::<usize>();
  let mut lengths = spans
    .iter()
    .map(|span| span.message_count)
    .collect::<Vec<_>>();
  lengths.sort_unstable();
  let singletons = lengths.iter().filter(|&&len| len == 1).count();
  let short = lengths.iter().filter(|&&len| len <= 3).count();
  let average = if spans.is_empty() {
    0.0
  } else {
    total_messages as f32 / spans.len() as f32
  };

  let mode_label = match mode {
    DebugSegmentationMode::Deterministic => "deterministic",
    DebugSegmentationMode::PlannerOnly => "planner-only",
    DebugSegmentationMode::EmbeddingPlanner => "embedding-planner",
    DebugSegmentationMode::FullLlm => "full-llm",
  };
  println!("mode: {mode_label}");
  println!("spans: {}", spans.len());
  println!("messages: {total_messages}");
  println!("avg_messages: {average:.2}");
  println!("median_messages: {:.2}", median(&lengths));
  println!("p90_messages: {}", percentile(&lengths, 0.9));
  println!("singleton_spans: {singletons}");
  println!("short_spans_le_3: {short}");
  println!(
    "deterministic_candidates: {}",
    trace.deterministic_candidates.len()
  );
  println!("planned_boundaries: {}", trace.planned_boundaries.len());
  println!("merged_boundaries: {}", trace.merged_boundaries.len());
  println!();

  for (index, span) in spans.iter().enumerate() {
    println!(
      "#{index:03} seq {}..{} messages={} reason={} surprise={} time={}..{}",
      span.start_seq,
      span.end_seq,
      span.message_count,
      span.boundary_reason,
      span.surprise_level,
      span.start_at.to_rfc3339(),
      span.end_at.to_rfc3339(),
    );
    println!("  first: {}", span.first_message);
    if span.last_message != span.first_message {
      println!("  last:  {}", span.last_message);
    }
    if detail {
      println!("  messages:");
      for message in &span.messages {
        println!(
          "    - seq {} | {} | {} | {}",
          message.seq,
          message.timestamp.to_rfc3339(),
          message.role,
          message.content,
        );
      }
    }
  }
}

fn print_trace(trace: &DebugSegmentationTrace) {
  println!("deterministic candidates:");
  for boundary in &trace.deterministic_candidates {
    println!(
      "  unit={} seq {} after {} score={:.2} gap={:.2} semantic={:.2} prior={:.2} penalty={:.2}",
      boundary.next_unit_index,
      boundary.next_seq,
      boundary.previous_seq,
      boundary.candidate_score,
      boundary.time_gap,
      boundary.semantic_drop,
      boundary.online_surprise_prior,
      boundary.micro_exchange_penalty,
    );
  }

  println!("\nplanned boundaries:");
  for boundary in &trace.planned_boundaries {
    println!(
      "  unit={} seq {} after {} reason={} surprise={} score={:.2} gap={:.2} semantic={:.2} prior={:.2} penalty={:.2}",
      boundary.next_unit_index,
      boundary.next_seq,
      boundary.previous_seq,
      boundary.boundary_reason.as_deref().unwrap_or("unknown"),
      boundary.surprise_level.as_deref().unwrap_or("unknown"),
      boundary.candidate_score,
      boundary.time_gap,
      boundary.semantic_drop,
      boundary.online_surprise_prior,
      boundary.micro_exchange_penalty,
    );
  }

  println!("\nmerged boundaries:");
  for boundary in &trace.merged_boundaries {
    println!(
      "  unit={} seq {} after {} reason={} surprise={} score={:.2} gap={:.2} semantic={:.2} prior={:.2} penalty={:.2}",
      boundary.next_unit_index,
      boundary.next_seq,
      boundary.previous_seq,
      boundary.boundary_reason.as_deref().unwrap_or("unknown"),
      boundary.surprise_level.as_deref().unwrap_or("unknown"),
      boundary.candidate_score,
      boundary.time_gap,
      boundary.semantic_drop,
      boundary.online_surprise_prior,
      boundary.micro_exchange_penalty,
    );
  }
  println!();
}

#[tokio::main]
async fn main() -> ExitCode {
  let args = match parse_args() {
    Ok(args) => args,
    Err(message) => {
      eprintln!("{message}");
      return ExitCode::from(2);
    }
  };

  let input = match read_input(args.input_path.as_deref()) {
    Ok(input) => input,
    Err(message) => {
      eprintln!("{message}");
      return ExitCode::from(1);
    }
  };

  let messages = match serde_json::from_str::<Vec<Message>>(&input) {
    Ok(messages) => messages,
    Err(error) => {
      eprintln!("failed to parse messages JSON: {error}");
      return ExitCode::from(1);
    }
  };

  let trace = if args.trace || args.detail || !args.json {
    match debug_segment_messages_with_trace(messages, args.mode).await {
      Ok(trace) => trace,
      Err(error) => {
        eprintln!("segmentation failed: {error}");
        return ExitCode::from(1);
      }
    }
  } else {
    match debug_segment_messages(messages, args.mode).await {
      Ok(spans) => DebugSegmentationTrace {
        deterministic_candidates: Vec::new(),
        planned_boundaries: Vec::new(),
        merged_boundaries: Vec::new(),
        spans,
      },
      Err(error) => {
        eprintln!("segmentation failed: {error}");
        return ExitCode::from(1);
      }
    }
  };

  if args.json {
    match serde_json::to_string_pretty(&trace.spans) {
      Ok(output) => println!("{output}"),
      Err(error) => {
        eprintln!("failed to serialize spans: {error}");
        return ExitCode::from(1);
      }
    }
  } else {
    if args.trace {
      print_trace(&trace);
    }
    print_summary(&trace, args.mode, args.detail);
  }

  ExitCode::SUCCESS
}
