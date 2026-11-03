//! `ProgressEvent` construction and publication.
//!
//! Progress travels as serialized protobuf bytes over the `events:progress`
//! Pub/Sub channel; the API decodes each event and fans JSON out to browser
//! WebSockets. Every stage change in every service goes through here so the
//! wire format stays identical everywhere.

use prost::Message;

use crate::error::AppResult;
use crate::streams::{Streams, PROGRESS_CHANNEL};
use crate::tender::common::v1::ProgressEvent;

/// Everything a service knows about one progress step.
#[derive(Debug, Clone)]
pub struct EventSpec {
    pub event_type: String,
    pub tender_id: String,
    pub task_id: String,
    pub run_id: String,
    pub stage: String,
    pub progress: f64,
    pub message: String,
    /// "info" | "success" | "warning" | "error"
    pub level: String,
}

impl EventSpec {
    pub fn new(event_type: impl Into<String>, tender_id: impl Into<String>) -> Self {
        Self {
            event_type: event_type.into(),
            tender_id: tender_id.into(),
            task_id: String::new(),
            run_id: String::new(),
            stage: String::new(),
            progress: 0.0,
            message: String::new(),
            level: "info".to_string(),
        }
    }

    pub fn task(mut self, task_id: impl Into<String>) -> Self {
        self.task_id = task_id.into();
        self
    }

    pub fn run(mut self, run_id: impl Into<String>) -> Self {
        self.run_id = run_id.into();
        self
    }

    pub fn stage(mut self, stage: impl Into<String>) -> Self {
        self.stage = stage.into();
        self
    }

    pub fn progress(mut self, progress: f64) -> Self {
        self.progress = progress.clamp(0.0, 1.0);
        self
    }

    pub fn message(mut self, message: impl Into<String>) -> Self {
        self.message = message.into();
        self
    }

    pub fn level(mut self, level: impl Into<String>) -> Self {
        self.level = level.into();
        self
    }

    pub fn build(self) -> ProgressEvent {
        ProgressEvent {
            event_id: uuid::Uuid::new_v4().to_string(),
            event_type: self.event_type,
            tender_id: self.tender_id,
            task_id: self.task_id,
            run_id: self.run_id,
            timestamp_unix_ms: chrono::Utc::now().timestamp_millis(),
            stage: self.stage,
            progress: self.progress,
            message: self.message,
            level: self.level,
            metadata: std::collections::HashMap::new(),
        }
    }
}

/// Build and publish one progress event. Publishing failures never fail the
/// business operation: they are logged and the pipeline continues (progress
/// is advisory, state in PostgreSQL is authoritative).
pub async fn publish(streams: &Streams, spec: EventSpec) -> AppResult<()> {
    let event = spec.build();
    let mut payload = Vec::with_capacity(event.encoded_len());
    if let Err(err) = event.encode(&mut payload) {
        tracing::warn!(error = %err, "progress event encode failed");
        return Ok(());
    }
    if let Err(err) = streams.publish(PROGRESS_CHANNEL, &payload).await {
        tracing::warn!(error = %err, "progress publish failed");
    }
    Ok(())
}
