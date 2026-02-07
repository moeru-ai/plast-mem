use std::time::Duration;

use apalis::prelude::{Monitor, TaskSink, WorkerBuilder};
use apalis_postgres::PostgresStorage;
use plast_mem_core::{
  EpisodicMemory, Message, MessageQueue, MessageRole, SegmentDecision, rule_segmenter,
};
use plast_mem_db_schema::episodic_memory;
use plast_mem_llm::{InputMessage, Role, decide_split};
use plast_mem_shared::AppError;
use sea_orm::{DatabaseConnection, EntityTrait};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug)]
pub struct WorkerError(pub AppError);

impl std::fmt::Display for WorkerError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    self.0.fmt(f)
  }
}

impl std::error::Error for WorkerError {}

impl From<AppError> for WorkerError {
  fn from(err: AppError) -> Self {
    Self(err)
  }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MessageQueueSegmentJob {
  pub conversation_id: Uuid,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CreateEpisodicMemoryJob {
  pub conversation_id: Uuid,
  pub segment_messages: Vec<Message>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum WorkerJob {
  Segment(MessageQueueSegmentJob),
  Create(CreateEpisodicMemoryJob),
}

fn to_input_messages(messages: &[Message]) -> Vec<InputMessage> {
  messages
    .iter()
    .map(|m| InputMessage {
      role: match m.role {
        MessageRole::User => Role::User,
        MessageRole::Assistant => Role::Assistant,
      },
      content: m.content.clone(),
    })
    .collect()
}

async fn handle_segment_job(
  job: MessageQueueSegmentJob,
  db: DatabaseConnection,
  mut backend: PostgresStorage<WorkerJob>,
) -> Result<(), WorkerError> {
  let queue = MessageQueue::get(job.conversation_id, &db).await?;
  let messages = queue.messages;
  let Some(incoming) = messages.last().cloned() else {
    return Ok(());
  };

  let recent = &messages[..messages.len().saturating_sub(1)];
  let decision = rule_segmenter(recent, &incoming);

  let should_split = match decision {
    SegmentDecision::Split => true,
    SegmentDecision::NoSplit => false,
    SegmentDecision::CallLlm => {
      let recent_input = to_input_messages(recent);
      let incoming_input = to_input_messages(std::slice::from_ref(&incoming))
        .pop()
        .expect("incoming message exists");
      decide_split(&recent_input, &incoming_input).await?
    }
  };

  if should_split {
    let segment_messages = recent.to_vec();

    // Atomically drain segment from queue front, preserving any newly pushed messages
    MessageQueue::drain(job.conversation_id, segment_messages.len(), &db).await?;

    backend
      .push(WorkerJob::Create(CreateEpisodicMemoryJob {
        conversation_id: job.conversation_id,
        segment_messages,
      }))
      .await
      .map_err(AppError::from)?;
  }

  Ok(())
}

async fn handle_create_job(
  job: CreateEpisodicMemoryJob,
  db: DatabaseConnection,
) -> Result<(), WorkerError> {
  let episodic = EpisodicMemory::new(job.conversation_id, job.segment_messages).await?;
  let model = episodic.to_model()?;
  let active_model: episodic_memory::ActiveModel = model.into();

  episodic_memory::Entity::insert(active_model)
    .exec(&db)
    .await
    .map_err(AppError::from)?;

  Ok(())
}

pub async fn worker(
  db: &DatabaseConnection,
  backend: PostgresStorage<WorkerJob>,
) -> Result<(), AppError> {
  let db = db.clone();

  Monitor::new()
    .register(move |_run_id| {
      let db = db.clone();
      let backend = backend.clone();

      WorkerBuilder::new("plast-mem-worker")
        .backend(backend.clone())
        .build(move |job: WorkerJob| {
          let db = db.clone();
          let backend = backend.clone();
          async move {
            match job {
              WorkerJob::Segment(job) => handle_segment_job(job, db, backend).await,
              WorkerJob::Create(job) => handle_create_job(job, db).await,
            }
          }
        })
    })
    .shutdown_timeout(Duration::from_secs(5))
    .run_with_signal(tokio::signal::ctrl_c())
    .await
    .map_err(|err| AppError::from(anyhow::Error::new(err)))?;

  Ok(())
}
