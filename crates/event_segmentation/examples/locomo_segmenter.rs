use std::{collections::BTreeMap, env, fs, path::PathBuf};

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Duration, NaiveDateTime, TimeZone, Utc};
use plastmem_event::{Event, EventData, EventDataToString, MessageEventData, MessageEventRole};
use plastmem_event_segmentation::{EventSegment, EventSegmenter};
use serde::Deserialize;
use tracing::Level;
use tracing_subscriber::FmtSubscriber;

const DEFAULT_DATA_FILE: &str = "benchmarks/locomo/data/locomo10.json";
const TURN_INTERVAL_MINS: i64 = 1;

#[derive(Debug, Deserialize)]
struct DialogTurn {
  #[serde(default)]
  blip_caption: Option<String>,
  #[serde(default)]
  query: Option<String>,
  #[serde(default)]
  search_query: Option<String>,
  speaker: String,
  text: String,
}

#[derive(Debug, Deserialize)]
struct LoCoMoSample {
  conversation: BTreeMap<String, serde_json::Value>,
  sample_id: String,
}

#[derive(Debug)]
struct OrderedSession {
  date: Option<DateTime<Utc>>,
  turns: Vec<DialogTurn>,
}

#[derive(Debug)]
struct Config {
  data_file: PathBuf,
  sample_id: Option<String>,
  sample_index: usize,
  print_events: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
  init_tracing();

  let config = parse_args()?;
  let samples = load_samples(&config.data_file)?;
  let sample = select_sample(&samples, &config)?;
  let events = build_events(sample)?;

  if events.is_empty() {
    return Err(anyhow!("selected sample contains no dialog turns"));
  }

  eprintln!(
    "sample_id={} events={} data_file={}",
    sample.sample_id,
    events.len(),
    config.data_file.display()
  );

  if config.print_events {
    for (index, event) in events.iter().enumerate() {
      eprintln!(
        "event[{index:03}] {} {}",
        event.timestamp.format("%Y-%m-%dT%H:%M:%SZ"),
        event.data.to_string_without_timestamp()
      );
    }
    eprintln!();
  }

  let segments = EventSegmenter::segment(&events)
    .await
    .map_err(|err| anyhow!(err.to_string()))?;
  write_segments_json(&segments)?;

  Ok(())
}

fn init_tracing() {
  let subscriber = FmtSubscriber::builder()
    .with_max_level(Level::WARN)
    .with_writer(std::io::stderr)
    .with_target(false)
    .without_time()
    .finish();

  let _ = tracing::subscriber::set_global_default(subscriber);
}

fn write_segments_json(segments: &[EventSegment]) -> Result<()> {
  serde_json::to_writer(std::io::stdout(), segments)
    .context("failed to serialize event segments to stdout")?;
  println!();
  Ok(())
}

fn parse_args() -> Result<Config> {
  let mut data_file = PathBuf::from(DEFAULT_DATA_FILE);
  let mut sample_id = None;
  let mut sample_index = 0usize;
  let mut print_events = false;

  let mut args = env::args().skip(1);
  while let Some(arg) = args.next() {
    match arg.as_str() {
      "--data-file" => {
        let value = args
          .next()
          .ok_or_else(|| anyhow!("--data-file requires a path"))?;
        data_file = PathBuf::from(value);
      }
      "--sample-id" => {
        sample_id = Some(
          args
            .next()
            .ok_or_else(|| anyhow!("--sample-id requires a value"))?,
        );
      }
      "--sample-index" => {
        let value = args
          .next()
          .ok_or_else(|| anyhow!("--sample-index requires a value"))?;
        sample_index = value
          .parse::<usize>()
          .with_context(|| format!("invalid --sample-index value: {value}"))?;
      }
      "--print-events" => {
        print_events = true;
      }
      "--help" | "-h" => {
        print_usage();
        std::process::exit(0);
      }
      other => {
        return Err(anyhow!("unknown argument: {other}"));
      }
    }
  }

  Ok(Config {
    data_file,
    sample_id,
    sample_index,
    print_events,
  })
}

fn print_usage() {
  eprintln!(
    "Usage: cargo run -p plastmem_event_segmentation --example locomo_segmenter -- [options]\n\
     \n\
     Options:\n\
       --data-file <path>      LoCoMo JSON file path (default: {DEFAULT_DATA_FILE})\n\
       --sample-id <id>        Select sample by sample_id\n\
       --sample-index <n>      Select sample by zero-based index when --sample-id is omitted\n\
       --print-events          Print flattened events before segmentation\n\
       -h, --help              Show this help"
  );
}

fn load_samples(path: &PathBuf) -> Result<Vec<LoCoMoSample>> {
  let raw = fs::read_to_string(path)
    .with_context(|| format!("failed to read LoCoMo data file: {}", path.display()))?;
  serde_json::from_str(&raw)
    .with_context(|| format!("failed to parse LoCoMo JSON from {}", path.display()))
}

fn select_sample<'a>(samples: &'a [LoCoMoSample], config: &Config) -> Result<&'a LoCoMoSample> {
  if let Some(sample_id) = &config.sample_id {
    return samples
      .iter()
      .find(|sample| &sample.sample_id == sample_id)
      .ok_or_else(|| anyhow!("sample_id not found: {sample_id}"));
  }

  samples
    .get(config.sample_index)
    .ok_or_else(|| anyhow!("sample_index out of range: {}", config.sample_index))
}

fn build_events(sample: &LoCoMoSample) -> Result<Vec<Event>> {
  let sessions = get_ordered_sessions(sample)?;
  let mut events = Vec::new();

  for session in sessions {
    for (turn_index, turn) in session.turns.iter().enumerate() {
      let text = turn.text.trim();
      if text.is_empty() {
        continue;
      }

      let mut parts = vec![text.to_owned()];
      append_image_context(
        &mut parts,
        turn.blip_caption.as_deref(),
        turn.query.as_deref().or(turn.search_query.as_deref()),
      );
      let content = parts.join(" ");
      let timestamp = get_turn_timestamp(session.date, turn_index)?;
      let role = parse_role(&turn.speaker);

      events.push(Event::new(
        EventData::Message(MessageEventData { role, content }),
        timestamp,
        None,
      ));
    }
  }

  Ok(events)
}

fn get_ordered_sessions(sample: &LoCoMoSample) -> Result<Vec<OrderedSession>> {
  let mut sessions = Vec::new();

  for session_number in 1..=100 {
    let turns_key = format!("session_{session_number}");
    let Some(turns_value) = sample.conversation.get(&turns_key) else {
      break;
    };

    let turns: Vec<DialogTurn> = serde_json::from_value(turns_value.clone())
      .with_context(|| format!("failed to parse {turns_key}"))?;
    let date = sample
      .conversation
      .get(&format!("session_{session_number}_date_time"))
      .and_then(|value| value.as_str())
      .map(parse_session_date)
      .transpose()?;

    sessions.push(OrderedSession { date, turns });
  }

  Ok(sessions)
}

fn get_turn_timestamp(
  session_date: Option<DateTime<Utc>>,
  turn_index: usize,
) -> Result<DateTime<Utc>> {
  if let Some(session_date) = session_date {
    let offset = i64::try_from(turn_index).context("turn index overflow")?;
    return Ok(session_date + Duration::minutes(offset * TURN_INTERVAL_MINS));
  }

  let base = Utc
    .with_ymd_and_hms(2023, 1, 1, 0, 0, 0)
    .single()
    .ok_or_else(|| anyhow!("failed to construct fallback timestamp"))?;
  let offset = i64::try_from(turn_index).context("turn index overflow")?;
  Ok(base + Duration::minutes(offset * TURN_INTERVAL_MINS))
}

fn parse_role(value: &str) -> MessageEventRole {
  match value.trim().to_ascii_lowercase().as_str() {
    "user" | "speaker1" => MessageEventRole::User,
    "assistant" | "speaker2" => MessageEventRole::Assistant,
    other => MessageEventRole::Custom(other.to_owned()),
  }
}

fn append_image_context(parts: &mut Vec<String>, caption: Option<&str>, query: Option<&str>) {
  let Some(caption) = caption.map(str::trim).filter(|caption| !caption.is_empty()) else {
    return;
  };

  if let Some(query) = query.map(str::trim).filter(|query| !query.is_empty()) {
    parts.push(format!(
      "[Image: {caption}; Image Retrieval Keywords: {query}]"
    ));
    return;
  }

  parts.push(format!("[Image: {caption}]"));
}

fn parse_session_date(value: &str) -> Result<DateTime<Utc>> {
  let value = value.trim();
  let naive = NaiveDateTime::parse_from_str(value, "%I:%M %P on %-d %B, %Y")
    .or_else(|_| NaiveDateTime::parse_from_str(value, "%I:%M %P on %d %B, %Y"))
    .with_context(|| format!("failed to parse session date: {value}"))?;
  Ok(Utc.from_utc_datetime(&naive))
}
