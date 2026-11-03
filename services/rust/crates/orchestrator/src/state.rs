//! Orchestrator state and the only code path that writes `tenders.status`.
//!
//! * [`OrchestratorState::apply`] performs one validated transition under a
//!   row lock (`SELECT ... FOR UPDATE`), so concurrent handlers can never
//!   interleave into an illegal state.
//! * [`OrchestratorState::advance`] walks the happy-path chain stage by stage
//!   to an aggregate milestone derived from the run's tasks.
//! * [`aggregate_target`] is a pure function: given task statuses it returns
//!   the furthest tender stage that is fully reached.

use std::sync::Arc;

use chrono::Utc;
use prost::Message;
use serde_json::Value as Json;
use sqlx::FromRow;
use uuid::Uuid;

use tender_common::config::Config;
use tender_common::error::{AppError, AppResult};
use tender_common::progress::{self, EventSpec};
use tender_common::streams::{self, Streams};

use crate::fsm::{Status, ALL_STATUSES};
use crate::tender::common::v1::JobEnvelope;
use crate::tender::matching::v1::matching_service_client::MatchingServiceClient;
use crate::tender::reports::v1::GenerateReportJob;

/// Result of handling one stream message.
pub enum Outcome {
    /// Handled (or intentionally ignored) — acknowledge the message.
    Done,
}

#[derive(Debug, Clone, FromRow)]
pub struct TaskRow {
    pub id: Uuid,
    pub component_id: Uuid,
    pub status: String,
}

#[derive(Clone)]
pub struct OrchestratorState {
    pub cfg: Arc<Config>,
    pub pg: sqlx::PgPool,
    pub streams: Streams,
    pub consumer: String,
}

impl OrchestratorState {
    pub fn new(cfg: Arc<Config>, pg: sqlx::PgPool, streams: Streams, consumer: String) -> Self {
        Self {
            cfg,
            pg,
            streams,
            consumer,
        }
    }

    // ------------------------------------------------------------------
    // FSM
    // ------------------------------------------------------------------

    pub async fn tender_status(&self, tender_id: Uuid) -> AppResult<Status> {
        let value: Option<String> = sqlx::query_scalar("SELECT status FROM tenders WHERE id = $1")
            .bind(tender_id)
            .fetch_optional(&self.pg)
            .await?;
        let value = value.ok_or_else(|| AppError::NotFound(format!("tender {tender_id}")))?;
        Status::parse(&value)
            .ok_or_else(|| AppError::internal(format!("unknown tender status '{value}'")))
    }

    /// One validated transition under a row lock. Same-state is a no-op.
    pub async fn apply(&self, tender_id: Uuid, next: Status) -> AppResult<Status> {
        let mut tx = self.pg.begin().await?;
        let current_value: Option<String> = sqlx::query_scalar(
            "SELECT status FROM tenders WHERE id = $1 AND deleted_at IS NULL FOR UPDATE",
        )
        .bind(tender_id)
        .fetch_optional(&mut *tx)
        .await?;
        let current_value =
            current_value.ok_or_else(|| AppError::NotFound(format!("tender {tender_id}")))?;
        let current = Status::parse(&current_value).ok_or_else(|| {
            AppError::internal(format!("unknown tender status '{current_value}'"))
        })?;
        if current == next {
            tx.commit().await?;
            return Ok(current);
        }
        let applied = current
            .transition_to(next)
            .map_err(|err| AppError::Conflict(err.to_string()))?;
        sqlx::query("UPDATE tenders SET status = $2, updated_at = now() WHERE id = $1")
            .bind(tender_id)
            .bind(applied.as_str())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        tracing::info!(
            %tender_id,
            from = current.as_str(),
            to = applied.as_str(),
            "tender transition"
        );
        Ok(applied)
    }

    /// Walk the pipeline forward until `target` is reached (never backwards,
    /// never past it). Terminal tenders are left untouched.
    pub async fn advance(&self, tender_id: Uuid, target: Status) -> AppResult<Status> {
        let mut current = self.tender_status(tender_id).await?;
        if current.is_terminal() || current == target {
            return Ok(current);
        }
        let target_index = stage_index(target);
        let mut guard = 0u8;
        while current != target {
            let Some(next) = current.successor() else {
                break;
            };
            if stage_index(next) > target_index {
                break;
            }
            current = self.apply(tender_id, next).await?;
            guard += 1;
            if guard > 16 {
                break;
            }
        }
        if current != target && Status::can_transition(current, target) {
            current = self.apply(tender_id, target).await?;
        }
        Ok(current)
    }

    pub async fn fail(&self, tender_id: Uuid, message: &str) -> AppResult<()> {
        let current = self.tender_status(tender_id).await?;
        if current.is_terminal() {
            return Ok(());
        }
        self.apply(tender_id, Status::Failed).await?;
        sqlx::query("UPDATE tenders SET error_message = $2 WHERE id = $1")
            .bind(tender_id)
            .bind(message)
            .execute(&self.pg)
            .await?;
        sqlx::query(
            "UPDATE search_runs SET status = 'FAILED', error_message = $2, finished_at = now()
             WHERE tender_id = $1 AND status IN ('QUEUED','RUNNING')",
        )
        .bind(tender_id)
        .bind(message)
        .execute(&self.pg)
        .await?;
        sqlx::query(
            "UPDATE search_tasks SET status = 'CANCELLED', finished_at = now()
             WHERE run_id IN (SELECT id FROM search_runs WHERE tender_id = $1)
               AND status NOT IN ('DONE','FAILED','CANCELLED')",
        )
        .bind(tender_id)
        .execute(&self.pg)
        .await?;
        self.activity(tender_id, "tender.failed", message, serde_json::json!({}))
            .await?;
        self.progress(tender_id, None, None, Status::Failed, 1.0, "error", message)
            .await;
        Ok(())
    }

    pub async fn cancel(&self, tender_id: Uuid, reason: &str) -> AppResult<()> {
        let current = self.tender_status(tender_id).await?;
        if current.is_terminal() {
            return Ok(());
        }
        self.apply(tender_id, Status::Cancelled).await?;
        sqlx::query(
            "UPDATE search_runs SET status = 'CANCELLED', cancelled_at = now(), finished_at = now()
             WHERE tender_id = $1 AND status IN ('QUEUED','RUNNING')",
        )
        .bind(tender_id)
        .execute(&self.pg)
        .await?;
        sqlx::query(
            "UPDATE search_tasks SET status = 'CANCELLED', finished_at = now()
             WHERE run_id IN (SELECT id FROM search_runs WHERE tender_id = $1)
               AND status NOT IN ('DONE','FAILED','CANCELLED')",
        )
        .bind(tender_id)
        .execute(&self.pg)
        .await?;
        self.activity(tender_id, "tender.cancelled", reason, serde_json::json!({}))
            .await?;
        self.progress(
            tender_id,
            None,
            None,
            Status::Cancelled,
            1.0,
            "warning",
            reason,
        )
        .await;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Milestones
    // ------------------------------------------------------------------

    pub async fn tasks(&self, run_id: Uuid) -> AppResult<Vec<TaskRow>> {
        let tasks = sqlx::query_as::<_, TaskRow>(
            "SELECT id, component_id, status FROM search_tasks WHERE run_id = $1 ORDER BY created_at, id",
        )
        .bind(run_id)
        .fetch_all(&self.pg)
        .await?;
        Ok(tasks)
    }

    /// Recompute the aggregate tender stage for a run and, once every task is
    /// final, queue report generation exactly once.
    pub async fn refresh_milestone(&self, tender_id: Uuid, run_id: Uuid) -> AppResult<()> {
        let tasks = self.tasks(run_id).await?;
        let current = self.tender_status(tender_id).await?;
        if current.is_terminal() {
            return Ok(());
        }
        let target = aggregate_target(&tasks);
        let reached = self.advance(tender_id, target).await?;
        if target == Status::ReportGeneration && reached == Status::ReportGeneration {
            self.enqueue_report(tender_id, run_id).await?;
        }
        Ok(())
    }

    pub async fn enqueue_report(&self, tender_id: Uuid, run_id: Uuid) -> AppResult<()> {
        let key = format!("report:queued:{run_id}");
        if !self.streams.set_nx(&key, 86_400).await? {
            return Ok(());
        }
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
            formats: Vec::new(), // all formats
        };
        self.streams
            .xadd_kind(streams::jobs::REPORTS, "report", &job.encode_to_vec())
            .await?;
        self.activity(
            tender_id,
            "report.queued",
            "report generation queued",
            serde_json::json!({ "run_id": run_id.to_string() }),
        )
        .await?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Small helpers
    // ------------------------------------------------------------------

    pub async fn activity(
        &self,
        tender_id: Uuid,
        event_type: &str,
        message: &str,
        metadata: Json,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO activity_logs (tender_id, user_id, event_type, message, metadata)
             VALUES ($1, NULL, $2, $3, $4)",
        )
        .bind(tender_id)
        .bind(event_type)
        .bind(message)
        .bind(metadata)
        .execute(&self.pg)
        .await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn progress(
        &self,
        tender_id: Uuid,
        run_id: Option<Uuid>,
        task_id: Option<Uuid>,
        stage: Status,
        value: f64,
        level: &str,
        message: impl Into<String>,
    ) {
        let mut spec = EventSpec::new("pipeline.progress", tender_id.to_string())
            .stage(stage.as_str())
            .progress(value)
            .level(level)
            .message(message);
        if let Some(run_id) = run_id {
            spec = spec.run(run_id.to_string());
        }
        if let Some(task_id) = task_id {
            spec = spec.task(task_id.to_string());
        }
        if let Err(err) = progress::publish(&self.streams, spec).await {
            tracing::warn!(error = %err, "progress publish failed");
        }
    }

    pub async fn matching_client(
        &self,
    ) -> AppResult<MatchingServiceClient<tonic::transport::Channel>> {
        MatchingServiceClient::connect(self.cfg.matching_grpc_url.clone())
            .await
            .map_err(|err| AppError::Upstream(format!("matching service unavailable: {err}")))
    }
}

/// Position of a status in the pipeline order; terminal statuses sort last.
pub fn stage_index(status: Status) -> usize {
    ALL_STATUSES
        .iter()
        .position(|value| *value == status.as_str())
        .unwrap_or(usize::MAX)
}

/// The furthest tender stage that every non-cancelled task has reached.
///
/// Task statuses: QUEUED, SEARCHING, FETCHING, EXTRACTING, MATCHING, DONE,
/// FAILED, CANCELLED.
pub fn aggregate_target(tasks: &[TaskRow]) -> Status {
    let active: Vec<&str> = tasks
        .iter()
        .map(|task| task.status.as_str())
        .filter(|status| *status != "CANCELLED")
        .collect();
    if active.is_empty() {
        return Status::ReportGeneration;
    }
    let all_in = |allowed: &[&str]| active.iter().all(|status| allowed.contains(status));
    if all_in(&["DONE", "FAILED"]) {
        return Status::ReportGeneration;
    }
    if all_in(&["MATCHING", "DONE", "FAILED"]) {
        return Status::Ranking;
    }
    if all_in(&["EXTRACTING", "MATCHING", "DONE", "FAILED"]) {
        return Status::ProductExtraction;
    }
    if all_in(&["FETCHING", "EXTRACTING", "MATCHING", "DONE", "FAILED"]) {
        return Status::Fetching;
    }
    Status::Searching
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(status: &str) -> TaskRow {
        TaskRow {
            id: Uuid::new_v4(),
            component_id: Uuid::new_v4(),
            status: status.to_string(),
        }
    }

    #[test]
    fn all_searching_stays_in_searching() {
        assert_eq!(
            aggregate_target(&[task("SEARCHING"), task("QUEUED")]),
            Status::Searching
        );
    }

    #[test]
    fn all_fetched_advances_to_fetching() {
        assert_eq!(
            aggregate_target(&[task("FETCHING"), task("EXTRACTING")]),
            Status::Fetching
        );
    }

    #[test]
    fn all_extracting_advances_to_product_extraction() {
        assert_eq!(
            aggregate_target(&[task("EXTRACTING"), task("MATCHING")]),
            Status::ProductExtraction
        );
    }

    #[test]
    fn all_matching_advances_to_ranking() {
        assert_eq!(
            aggregate_target(&[task("MATCHING"), task("DONE")]),
            Status::Ranking
        );
    }

    #[test]
    fn all_final_queues_reports() {
        assert_eq!(
            aggregate_target(&[task("DONE"), task("FAILED")]),
            Status::ReportGeneration
        );
    }

    #[test]
    fn cancelled_tasks_are_ignored() {
        assert_eq!(
            aggregate_target(&[task("DONE"), task("CANCELLED")]),
            Status::ReportGeneration
        );
        assert_eq!(
            aggregate_target(&[task("CANCELLED")]),
            Status::ReportGeneration
        );
    }

    #[test]
    fn mixed_progress_takes_the_slowest_task() {
        assert_eq!(
            aggregate_target(&[task("SEARCHING"), task("DONE")]),
            Status::Searching
        );
    }

    #[test]
    fn stage_index_orders_the_pipeline() {
        assert!(stage_index(Status::Created) < stage_index(Status::Searching));
        assert!(stage_index(Status::Searching) < stage_index(Status::ReportGeneration));
    }
}
