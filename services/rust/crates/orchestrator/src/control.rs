//! `jobs:control` handler — API commands to the orchestrator.
//!
//! Actions live in `JobEnvelope.metadata["action"]`:
//! `start_run`, `cancel`, `regenerate_report`.

use chrono::Utc;
use futures::future::BoxFuture;
use prost::Message;
use uuid::Uuid;

use tender_common::error::AppResult;
use tender_common::streams::{self, StreamMessage};

use crate::fsm::Status;
use crate::state::{OrchestratorState, Outcome};
use crate::tender::common::v1::JobEnvelope;
use crate::tender::reports::v1::GenerateReportJob;
use crate::tender::search::v1::SearchPositionJob;

pub fn handle_control<'a>(
    state: &'a OrchestratorState,
    message: &'a StreamMessage,
) -> BoxFuture<'a, AppResult<Outcome>> {
    Box::pin(async move {
        let envelope = match JobEnvelope::decode(message.payload.as_slice()) {
            Ok(envelope) => envelope,
            Err(err) => {
                tracing::error!(error = %err, "undecodable control JobEnvelope — dropping");
                return Ok(Outcome::Done);
            }
        };
        let action = envelope.metadata.get("action").cloned().unwrap_or_default();

        match action.as_str() {
            "start_run" => start_run(state, &envelope).await,
            "cancel" => {
                let Ok(tender_id) = Uuid::parse_str(&envelope.tender_id) else {
                    tracing::error!("cancel command without a valid tender id");
                    return Ok(Outcome::Done);
                };
                state.cancel(tender_id, "cancelled by the operator").await?;
                Ok(Outcome::Done)
            }
            "documents_uploaded" => {
                let Ok(tender_id) = Uuid::parse_str(&envelope.tender_id) else {
                    tracing::error!("documents_uploaded without a valid tender id");
                    return Ok(Outcome::Done);
                };
                // Documents are already queued by the API; the orchestrator
                // only owns the FSM advance (Created -> Uploading ->
                // DocumentProcessing).
                state.advance(tender_id, Status::DocumentProcessing).await?;
                Ok(Outcome::Done)
            }
            "regenerate_report" => {
                let (Ok(tender_id), Ok(run_id)) = (
                    Uuid::parse_str(&envelope.tender_id),
                    Uuid::parse_str(&envelope.run_id),
                ) else {
                    tracing::error!("regenerate_report needs tender_id and run_id");
                    return Ok(Outcome::Done);
                };
                regenerate_report(state, tender_id, run_id).await?;
                Ok(Outcome::Done)
            }
            other => {
                tracing::warn!(action = other, "unknown control action — dropping");
                Ok(Outcome::Done)
            }
        }
    })
}

/// Start (or resume) a search run: queue one `jobs:search` message per
/// pending position. Idempotent: finished runs are ignored, task-level
/// deduplication lives in the search worker.
async fn start_run(state: &OrchestratorState, envelope: &JobEnvelope) -> AppResult<Outcome> {
    let (Ok(run_id), Ok(tender_id)) = (
        Uuid::parse_str(&envelope.run_id),
        Uuid::parse_str(&envelope.tender_id),
    ) else {
        tracing::error!("start_run needs a valid run_id and tender_id");
        return Ok(Outcome::Done);
    };

    let run_status: Option<String> =
        sqlx::query_scalar("SELECT status FROM search_runs WHERE id = $1")
            .bind(run_id)
            .fetch_optional(&state.pg)
            .await?;
    match run_status.as_deref() {
        Some("QUEUED") | Some("RUNNING") => {}
        Some(other) => {
            tracing::info!(%run_id, status = other, "run already finished; ignoring start_run");
            return Ok(Outcome::Done);
        }
        None => {
            tracing::error!(%run_id, "start_run for an unknown run");
            return Ok(Outcome::Done);
        }
    }

    sqlx::query(
        "UPDATE search_runs
         SET status = 'RUNNING', started_at = COALESCE(started_at, now()), updated_at = now()
         WHERE id = $1",
    )
    .bind(run_id)
    .execute(&state.pg)
    .await?;

    state.advance(tender_id, Status::Searching).await?;

    let tasks = state.tasks(run_id).await?;
    if tasks.is_empty() {
        tracing::warn!(%run_id, "run has no positions; nothing to search");
    }

    let mut queued = 0usize;
    for task in tasks.iter().filter(|task| task.status == "QUEUED") {
        sqlx::query(
            "UPDATE components SET status = 'SEARCHING'
             WHERE id = $1 AND status IN ('QUEUED','REVIEWED','EXTRACTED')",
        )
        .bind(task.component_id)
        .execute(&state.pg)
        .await?;

        let job = SearchPositionJob {
            envelope: Some(JobEnvelope {
                task_id: task.id.to_string(),
                tender_id: tender_id.to_string(),
                run_id: run_id.to_string(),
                created_at_unix_ms: Utc::now().timestamp_millis(),
                attempt: 1,
                metadata: Default::default(),
            }),
            component_id: task.component_id.to_string(),
        };
        state
            .streams
            .xadd_kind(streams::jobs::SEARCH, "search", &job.encode_to_vec())
            .await?;
        queued += 1;
    }

    state
        .activity(
            tender_id,
            "search.started",
            &format!("{queued} position(s) queued for search"),
            serde_json::json!({ "run_id": run_id.to_string(), "positions": queued }),
        )
        .await?;
    state
        .progress(
            tender_id,
            Some(run_id),
            None,
            Status::Searching,
            0.0,
            "info",
            format!("searching {queued} position(s)"),
        )
        .await;

    state.refresh_milestone(tender_id, run_id).await?;
    Ok(Outcome::Done)
}

/// Explicit operator request: rebuild the report set for a run.
/// Finished (terminal) tenders are immutable — a new run is required.
async fn regenerate_report(
    state: &OrchestratorState,
    tender_id: Uuid,
    run_id: Uuid,
) -> AppResult<()> {
    let current = state.tender_status(tender_id).await?;
    if current.is_terminal() {
        tracing::warn!(
            %tender_id,
            status = current.as_str(),
            "report regeneration requested for a finished tender; start a new run instead"
        );
        return Ok(());
    }
    state.advance(tender_id, Status::ReportGeneration).await?;

    let job = GenerateReportJob {
        envelope: Some(JobEnvelope {
            task_id: Uuid::new_v4().to_string(),
            tender_id: tender_id.to_string(),
            run_id: run_id.to_string(),
            created_at_unix_ms: Utc::now().timestamp_millis(),
            attempt: 1,
            metadata: Default::default(),
        }),
        tender_id: tender_id.to_string(),
        run_id: run_id.to_string(),
        formats: Vec::new(),
    };
    state
        .streams
        .xadd_kind(streams::jobs::REPORTS, "report", &job.encode_to_vec())
        .await?;
    state
        .activity(
            tender_id,
            "report.regenerated",
            "report generation re-queued by the operator",
            serde_json::json!({ "run_id": run_id.to_string() }),
        )
        .await?;
    state
        .progress(
            tender_id,
            Some(run_id),
            None,
            Status::ReportGeneration,
            0.0,
            "info",
            "report generation re-queued",
        )
        .await;
    Ok(())
}
