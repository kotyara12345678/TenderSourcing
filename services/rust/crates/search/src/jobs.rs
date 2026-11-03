//! `jobs:search` consumer-group worker: for one tender position it
//! generates queries, calls Keenable, prefilters results by relevance,
//! fetches the top pages, persists full provenance and publishes a notice
//! (`SearchPositionResult`) on `events:search` for the orchestrator.
//!
//! Delivery is at-least-once, so every position is guarded by a Redis
//! `SET NX` idempotency key derived from `JobEnvelope.task_id`; the key is
//! released when a retryable (infrastructure) failure occurs so the message
//! can be redelivered by `XAUTOCLAIM`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use prost::Message;
use serde_json::Value as Json;
use sha2::{Digest, Sha256};
use sqlx::FromRow;
use uuid::Uuid;

use tender_common::config::Config;
use tender_common::error::{AppError, AppResult};
use tender_common::progress::{self, EventSpec};
use tender_common::streams::{self, StreamMessage, Streams};

use crate::client::KeenableClient;
use crate::fetch::{truncate_chars, FetchBackend};
use crate::queries::{generate_queries, relevance_score, QueryCandidate, QueryInput};
use crate::tender::common::v1::JobEnvelope;
use crate::tender::search::v1::{SearchPositionJob, SearchPositionResult, SearchRequest};

/// Idempotency-key TTL: comfortably longer than any single tender run.
const DEDUP_TTL_SECONDS: u64 = 6 * 3600;
/// A message that keeps failing on infrastructure errors is dropped only
/// after this many attempts, and always with an ERROR log.
const MAX_ATTEMPTS: u32 = 5;
/// Claim messages abandoned by a dead consumer after this idle time.
const AUTOCLAIM_IDLE_MS: u64 = 600_000;
/// Pause before re-scanning messages deferred to another live consumer.
const DEFERRED_PAUSE_SECONDS: u64 = 30;
/// Pages scoring below this are not worth spending fetch credits on.
const MIN_RELEVANCE: f64 = 0.05;

/// What happened to one stream message.
pub enum Outcome {
    /// Fully handled — acknowledge it.
    Done,
    /// Another consumer holds the idempotency key and the task is still
    /// active: leave the message pending and retry later.
    Deferred,
}

#[derive(Debug, FromRow)]
struct TaskRow {
    status: String,
}

#[derive(Debug, FromRow)]
struct RunRow {
    status: String,
    queries_issued: i32,
}

#[derive(Debug, FromRow)]
struct ComponentRow {
    name: String,
    category: String,
    search_hints: Json,
}

#[derive(Debug, FromRow)]
struct RequirementRow {
    name: String,
    value_text: String,
    unit: String,
    data_type: String,
}

#[derive(Debug, FromRow)]
struct ResultRow {
    id: Uuid,
    url: String,
    title: String,
    snippet: String,
}

pub struct SearchWorker {
    cfg: Arc<Config>,
    pg: sqlx::PgPool,
    streams: Streams,
    client: Arc<KeenableClient>,
    backend: Arc<FetchBackend>,
    consumer: String,
    attempts: Mutex<HashMap<String, u32>>,
}

impl SearchWorker {
    pub fn new(
        cfg: Arc<Config>,
        pg: sqlx::PgPool,
        streams: Streams,
        client: Arc<KeenableClient>,
        backend: Arc<FetchBackend>,
        consumer: String,
    ) -> Self {
        Self {
            cfg,
            pg,
            streams,
            client,
            backend,
            consumer,
            attempts: Mutex::new(HashMap::new()),
        }
    }

    /// Blocking read loop: new messages, then crash-recovery claims.
    pub async fn run(self: Arc<Self>) -> AppResult<()> {
        tracing::info!(consumer = %self.consumer, "search worker loop started");
        loop {
            let mut batch = match self
                .streams
                .autoclaim(
                    streams::jobs::SEARCH,
                    streams::groups::SEARCH_WORKERS,
                    &self.consumer,
                    AUTOCLAIM_IDLE_MS,
                    8,
                )
                .await
            {
                Ok(messages) => messages,
                Err(err) => {
                    tracing::warn!(error = %err, "autoclaim failed; continuing with new messages");
                    Vec::new()
                }
            };

            match self
                .streams
                .read(
                    streams::jobs::SEARCH,
                    streams::groups::SEARCH_WORKERS,
                    &self.consumer,
                    4,
                    5_000,
                )
                .await
            {
                Ok(messages) => batch.extend(messages),
                Err(err) => {
                    tracing::error!(error = %err, "stream read failed; backing off");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
            }

            let mut deferred = 0usize;
            for message in &batch {
                match self.handle(message).await {
                    Ok(Outcome::Done) => {
                        self.ack(message).await?;
                        self.attempts
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .remove(&message.id);
                    }
                    Ok(Outcome::Deferred) => deferred += 1,
                    Err(err) => {
                        tracing::error!(message_id = %message.id, error = %err, "search job failed");
                        let attempt = {
                            let mut map = self
                                .attempts
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner());
                            let counter = map.entry(message.id.clone()).or_insert(0);
                            *counter += 1;
                            *counter
                        };
                        if attempt >= MAX_ATTEMPTS {
                            tracing::error!(
                                message_id = %message.id,
                                attempts = attempt,
                                "dropping search job after repeated infrastructure failures"
                            );
                            self.ack(message).await?;
                            self.attempts
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner())
                                .remove(&message.id);
                        }
                    }
                }
            }

            if deferred > 0 {
                tracing::info!(
                    deferred,
                    "positions are in flight elsewhere; pausing before the next claim scan"
                );
                tokio::time::sleep(Duration::from_secs(DEFERRED_PAUSE_SECONDS)).await;
            }
        }
    }

    async fn ack(&self, message: &StreamMessage) -> AppResult<()> {
        self.streams
            .ack(
                streams::jobs::SEARCH,
                streams::groups::SEARCH_WORKERS,
                std::slice::from_ref(&message.id),
            )
            .await?;
        Ok(())
    }

    async fn handle(&self, message: &StreamMessage) -> AppResult<Outcome> {
        let job = match SearchPositionJob::decode(message.payload.as_slice()) {
            Ok(job) => job,
            Err(err) => {
                tracing::error!(message_id = %message.id, error = %err, "undecodable SearchPositionJob — dropping");
                return Ok(Outcome::Done);
            }
        };
        let envelope = match job.envelope.clone() {
            Some(envelope) => envelope,
            None => {
                tracing::error!(message_id = %message.id, "SearchPositionJob without envelope — dropping");
                return Ok(Outcome::Done);
            }
        };
        let task_id = parse_uuid("task_id", &envelope.task_id)?;
        let key = format!("dedup:search:{task_id}");

        if !self.streams.set_nx(&key, DEDUP_TTL_SECONDS).await? {
            // Either completed earlier (ack is safe) or still running elsewhere.
            return match self.task_status(task_id).await? {
                Some(status) if is_terminal(&status) => Ok(Outcome::Done),
                Some(status) => {
                    tracing::info!(%task_id, status = %status, "position already in flight; deferring");
                    Ok(Outcome::Deferred)
                }
                None => Ok(Outcome::Done), // task row gone: nothing to do
            };
        }

        match self.process(&envelope, &job).await {
            Ok(()) => Ok(Outcome::Done),
            Err(err) if is_infra(&err) => {
                // Allow redelivery to re-enter the handler.
                self.streams.release(&key).await.ok();
                Err(err)
            }
            Err(err) => {
                // Business failure: the position cannot be searched. State is
                // persisted, the orchestrator is notified, and the message is
                // acknowledged so it is not retried.
                self.fail_position(&envelope, &job, &err).await?;
                Ok(Outcome::Done)
            }
        }
    }

    async fn task_status(&self, task_id: Uuid) -> AppResult<Option<String>> {
        let status =
            sqlx::query_scalar::<_, String>("SELECT status FROM search_tasks WHERE id = $1")
                .bind(task_id)
                .fetch_optional(&self.pg)
                .await?;
        Ok(status)
    }

    async fn process(&self, envelope: &JobEnvelope, job: &SearchPositionJob) -> AppResult<()> {
        let task_id = parse_uuid("task_id", &envelope.task_id)?;
        let run_id = parse_uuid("run_id", &envelope.run_id)?;
        let component_id = parse_uuid("component_id", &job.component_id)?;
        let tender_id = parse_uuid("tender_id", &envelope.tender_id)?;

        let task: TaskRow = sqlx::query_as("SELECT status FROM search_tasks WHERE id = $1")
            .bind(task_id)
            .fetch_optional(&self.pg)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("search task {task_id}")))?;
        if is_terminal(&task.status) {
            tracing::debug!(%task_id, status = %task.status, "task already terminal");
            return Ok(());
        }

        let run: RunRow =
            sqlx::query_as("SELECT status, queries_issued FROM search_runs WHERE id = $1")
                .bind(run_id)
                .fetch_optional(&self.pg)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("search run {run_id}")))?;
        if matches!(run.status.as_str(), "FAILED" | "CANCELLED") {
            self.cancel_task(task_id).await?;
            return Ok(());
        }

        let tender_status: Option<String> =
            sqlx::query_scalar("SELECT status FROM tenders WHERE id = $1")
                .bind(tender_id)
                .fetch_optional(&self.pg)
                .await?;
        if !matches!(
            tender_status.as_deref(),
            Some("SEARCH_QUEUED")
                | Some("SEARCHING")
                | Some("FETCHING")
                | Some("PRODUCT_EXTRACTION")
        ) {
            // The tender moved on (cancelled, failed, reset): spend nothing.
            self.cancel_task(task_id).await?;
            return Ok(());
        }

        let claimed = sqlx::query(
            "UPDATE search_tasks
             SET status = 'SEARCHING', attempts = attempts + 1,
                 started_at = COALESCE(started_at, now()), updated_at = now()
             WHERE id = $1 AND status NOT IN ('DONE','FAILED','CANCELLED')",
        )
        .bind(task_id)
        .execute(&self.pg)
        .await?;
        if claimed.rows_affected() == 0 {
            return Ok(());
        }

        let component: ComponentRow =
            sqlx::query_as("SELECT name, category, search_hints FROM components WHERE id = $1")
                .bind(component_id)
                .fetch_optional(&self.pg)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("component {component_id}")))?;

        let requirements: Vec<RequirementRow> = sqlx::query_as(
            "SELECT name, value_text, unit, data_type
             FROM requirements WHERE component_id = $1 ORDER BY name",
        )
        .bind(component_id)
        .fetch_all(&self.pg)
        .await?;
        let requirement_tuples: Vec<(String, String, String, String)> = requirements
            .iter()
            .map(|requirement| {
                (
                    requirement.name.clone(),
                    requirement.value_text.clone(),
                    requirement.unit.clone(),
                    requirement.data_type.clone(),
                )
            })
            .collect();

        // Query budget: global cap and the provider's monthly search limit.
        let used = run.queries_issued.max(0) as u32;
        let provider_cap = self.cfg.keenable_search_limit.min(u64::from(u32::MAX)) as u32;
        let total_cap = self.cfg.max_search_queries.min(provider_cap.max(1));
        let remaining = total_cap.saturating_sub(used) as usize;
        if remaining == 0 {
            return Err(AppError::Conflict(
                "search query budget for this tender is exhausted".into(),
            ));
        }

        let input = QueryInput {
            name: &component.name,
            category: &component.category,
            hints: &component.search_hints,
            requirements: &requirement_tuples,
        };
        let queries = generate_queries(&input, remaining);
        if queries.is_empty() {
            return Err(AppError::Validation(
                "component produced no search queries".into(),
            ));
        }

        let mut queries_issued = 0i32;
        let mut search_credits = 0i64;
        let mut stopped_on_rate_limit = false;

        for (index, candidate) in queries.iter().enumerate() {
            progress::publish(
                &self.streams,
                EventSpec::new("search.progress", tender_id.to_string())
                    .task(task_id.to_string())
                    .run(run_id.to_string())
                    .stage("SEARCHING")
                    .progress(index as f64 / queries.len() as f64)
                    .message(format!(
                        "query {}/{}: {}",
                        index + 1,
                        queries.len(),
                        candidate.query
                    )),
            )
            .await?;

            let request = SearchRequest {
                query: candidate.query.clone(),
                mode: self.cfg.keenable_search_mode.clone(),
                site: String::new(),
                published_after: String::new(),
                published_before: String::new(),
                snippet_max_length: 0,
                max_results: i32::from(self.cfg.fetch_max_per_component.max(1)),
            };

            match self.client.search(&request).await {
                Ok(response) => {
                    queries_issued += 1;
                    let results_count = response.results.len() as i32;
                    let query_id = self
                        .persist_query(task_id, candidate, index as i32, results_count, None)
                        .await?;

                    for (rank, result) in response.results.iter().enumerate() {
                        if !(result.url.starts_with("http://")
                            || result.url.starts_with("https://"))
                        {
                            continue;
                        }
                        sqlx::query(
                            "INSERT INTO search_results
                                (query_id, task_id, url, title, snippet, published_at, acquired_at, rank)
                             VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
                             ON CONFLICT (task_id, url) DO NOTHING",
                        )
                        .bind(query_id)
                        .bind(task_id)
                        .bind(&result.url)
                        .bind(truncate_chars(&result.title, 1000))
                        .bind(truncate_chars(&result.snippet, 4000))
                        .bind(parse_timestamp(&result.published_at))
                        .bind(parse_timestamp(&result.acquired_at))
                        .bind(rank as i32)
                        .execute(&self.pg)
                        .await?;
                    }

                    if let Some(usage) = &response.usage {
                        let sku = if usage.sku.is_empty() {
                            format!("search.{}", self.cfg.keenable_search_mode)
                        } else {
                            usage.sku.clone()
                        };
                        self.record_usage(
                            "keenable_search",
                            &sku,
                            tender_id,
                            run_id,
                            component_id,
                            task_id,
                            usage.credits,
                        )
                        .await?;
                        search_credits += usage.credits;
                    }
                }
                Err(err) => {
                    let message = err.to_string();
                    tracing::warn!(
                        query = %candidate.query,
                        error = %message,
                        "search query failed"
                    );
                    self.persist_query(task_id, candidate, index as i32, 0, Some(&message))
                        .await?;
                    if matches!(err, AppError::RateLimited(_)) {
                        stopped_on_rate_limit = true;
                        break;
                    }
                }
            }
        }

        if queries_issued == 0 {
            return Err(AppError::Upstream(if stopped_on_rate_limit {
                "keenable rate limit prevented all queries".into()
            } else {
                "every search query failed upstream".into()
            }));
        }

        // Prefilter: deterministic relevance scoring for the prefilter stage.
        let unscored: Vec<ResultRow> = sqlx::query_as(
            "SELECT id, url, title, snippet FROM search_results
             WHERE task_id = $1 AND relevance_score IS NULL",
        )
        .bind(task_id)
        .fetch_all(&self.pg)
        .await?;
        for row in &unscored {
            let score = relevance_score(&component.name, &row.title, &row.snippet);
            sqlx::query("UPDATE search_results SET relevance_score = $2 WHERE id = $1")
                .bind(row.id)
                .bind(score)
                .execute(&self.pg)
                .await?;
        }

        sqlx::query(
            "UPDATE search_tasks SET status = 'FETCHING', updated_at = now() WHERE id = $1",
        )
        .bind(task_id)
        .execute(&self.pg)
        .await?;

        let max_pages = i64::from(self.cfg.fetch_max_per_component.max(1));
        let candidates: Vec<ResultRow> = sqlx::query_as(
            "SELECT id, url, title, snippet FROM search_results
             WHERE task_id = $1 AND fetched = FALSE
               AND relevance_score IS NOT NULL AND relevance_score > $3
             ORDER BY relevance_score DESC NULLS LAST, created_at, id
             LIMIT $2",
        )
        .bind(task_id)
        .bind(max_pages)
        .bind(MIN_RELEVANCE)
        .fetch_all(&self.pg)
        .await?;

        let mut pages_fetched = 0i32;
        let mut fetch_credits = 0i64;
        let provider = self.backend.provider_name();

        for (index, candidate) in candidates.iter().enumerate() {
            progress::publish(
                &self.streams,
                EventSpec::new("search.progress", tender_id.to_string())
                    .task(task_id.to_string())
                    .run(run_id.to_string())
                    .stage("FETCHING")
                    .progress(index as f64 / candidates.len().max(1) as f64)
                    .message(format!(
                        "fetch {}/{}: {}",
                        index + 1,
                        candidates.len(),
                        candidate.url
                    )),
            )
            .await?;

            match self
                .backend
                .fetch(&candidate.url, self.cfg.fetch_max_chars, false)
                .await
            {
                Ok(page) => {
                    let content_hash = sha256_hex(page.content.as_bytes());
                    let content_type = if provider == "keenable" {
                        "text/markdown"
                    } else {
                        "text/html"
                    };
                    let size_bytes = page.content.len().min(i32::MAX as usize) as i32;
                    let source_id: Uuid = sqlx::query_scalar(
                        "INSERT INTO sources
                            (url, original_url, provider, content_hash, http_status,
                             content_type, title, content, snippet, size_bytes)
                         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)
                         ON CONFLICT (original_url, content_hash)
                         DO UPDATE SET retrieved_at = now()
                         RETURNING id",
                    )
                    .bind(&page.url)
                    .bind(&candidate.url)
                    .bind(provider)
                    .bind(&content_hash)
                    .bind(page.http_status)
                    .bind(content_type)
                    .bind(truncate_chars(&page.title, 1000))
                    .bind(&page.content)
                    .bind(truncate_chars(&candidate.snippet, 2000))
                    .bind(size_bytes)
                    .fetch_one(&self.pg)
                    .await?;

                    sqlx::query(
                        "UPDATE search_results SET fetched = TRUE, source_id = $2 WHERE id = $1",
                    )
                    .bind(candidate.id)
                    .bind(source_id)
                    .execute(&self.pg)
                    .await?;

                    pages_fetched += 1;
                    if provider == "keenable" && page.credits > 0 {
                        self.record_usage(
                            "keenable_fetch",
                            "fetch",
                            tender_id,
                            run_id,
                            component_id,
                            task_id,
                            page.credits,
                        )
                        .await?;
                        fetch_credits += page.credits;
                    }
                }
                Err(err) => {
                    // A single unreachable page never fails the position.
                    tracing::warn!(url = %candidate.url, error = %err, "page fetch failed; skipping");
                }
            }
        }

        let task_results: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM search_results WHERE task_id = $1")
                .bind(task_id)
                .fetch_one(&self.pg)
                .await?;

        sqlx::query(
            "UPDATE search_runs
             SET queries_issued = queries_issued + $2,
                 results_found = results_found + $3,
                 pages_fetched = pages_fetched + $4,
                 search_credits = search_credits + $5,
                 fetch_credits = fetch_credits + $6,
                 updated_at = now()
             WHERE id = $1",
        )
        .bind(run_id)
        .bind(queries_issued)
        .bind(task_results.min(i64::from(i32::MAX)) as i32)
        .bind(pages_fetched)
        .bind(search_credits)
        .bind(fetch_credits)
        .execute(&self.pg)
        .await?;

        // The task stays in FETCHING: the orchestrator owns the FSM and moves
        // it to EXTRACTING when the AI products job is queued.
        let notice = SearchPositionResult {
            envelope: Some(envelope.clone()),
            component_id: component_id.to_string(),
            queries_issued,
            results_found: task_results.min(i64::from(i32::MAX)) as i32,
            pages_fetched,
            search_credits,
            fetch_credits,
            success: true,
            error_message: String::new(),
        };
        self.streams
            .xadd_kind(streams::events::SEARCH, "search", &notice.encode_to_vec())
            .await?;

        progress::publish(
            &self.streams,
            EventSpec::new("search.completed", tender_id.to_string())
                .task(task_id.to_string())
                .run(run_id.to_string())
                .stage("FETCHING")
                .progress(1.0)
                .level("success")
                .message(format!(
                    "found {task_results} results, fetched {pages_fetched} pages"
                )),
        )
        .await?;

        Ok(())
    }

    async fn persist_query(
        &self,
        task_id: Uuid,
        candidate: &QueryCandidate,
        order: i32,
        results: i32,
        error: Option<&str>,
    ) -> AppResult<Uuid> {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO search_queries (task_id, query, kind, sort_order, results_count, error)
             VALUES ($1,$2,$3,$4,$5,$6)
             ON CONFLICT (task_id, query)
             DO UPDATE SET results_count = EXCLUDED.results_count, error = EXCLUDED.error
             RETURNING id",
        )
        .bind(task_id)
        .bind(&candidate.query)
        .bind(&candidate.kind)
        .bind(order)
        .bind(results)
        .bind(error)
        .fetch_one(&self.pg)
        .await?;
        Ok(id)
    }

    #[allow(clippy::too_many_arguments)]
    async fn record_usage(
        &self,
        service: &str,
        sku: &str,
        tender_id: Uuid,
        run_id: Uuid,
        component_id: Uuid,
        task_id: Uuid,
        credits: i64,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO api_usage
                (service, sku, tender_id, run_id, component_id, task_id, quantity, credits)
             VALUES ($1, $2, $3, $4, $5, $6, 1, $7::float8)",
        )
        .bind(service)
        .bind(sku)
        .bind(tender_id)
        .bind(run_id)
        .bind(component_id)
        .bind(task_id.to_string())
        .bind(credits as f64)
        .execute(&self.pg)
        .await?;
        Ok(())
    }

    async fn cancel_task(&self, task_id: Uuid) -> AppResult<()> {
        sqlx::query(
            "UPDATE search_tasks SET status = 'CANCELLED', finished_at = now()
             WHERE id = $1 AND status NOT IN ('DONE','FAILED','CANCELLED')",
        )
        .bind(task_id)
        .execute(&self.pg)
        .await?;
        Ok(())
    }

    async fn fail_position(
        &self,
        envelope: &JobEnvelope,
        job: &SearchPositionJob,
        err: &AppError,
    ) -> AppResult<()> {
        let task_id = parse_uuid("task_id", &envelope.task_id)?;
        let run_id = parse_uuid("run_id", &envelope.run_id)?;
        let component_id = parse_uuid("component_id", &job.component_id)?;
        let tender_id = parse_uuid("tender_id", &envelope.tender_id)?;
        let message = err.to_string();

        sqlx::query(
            "UPDATE search_tasks
             SET status = 'FAILED', last_error = $2, finished_at = now(), updated_at = now()
             WHERE id = $1 AND status NOT IN ('DONE','FAILED','CANCELLED')",
        )
        .bind(task_id)
        .bind(&message)
        .execute(&self.pg)
        .await?;

        sqlx::query(
            "UPDATE components SET status = 'FAILED', error_message = $2
             WHERE id = $1 AND status NOT IN ('MATCHED','SKIPPED')",
        )
        .bind(component_id)
        .bind(&message)
        .execute(&self.pg)
        .await?;

        sqlx::query(
            "UPDATE search_runs SET components_failed = components_failed + 1, updated_at = now()
             WHERE id = $1",
        )
        .bind(run_id)
        .execute(&self.pg)
        .await?;

        sqlx::query(
            "INSERT INTO activity_logs (tender_id, user_id, event_type, message, metadata)
             VALUES ($1, NULL, 'search.position_failed', $2, $3)",
        )
        .bind(tender_id)
        .bind(format!("position search failed: {message}"))
        .bind(serde_json::json!({
            "component_id": component_id.to_string(),
            "run_id": run_id.to_string(),
            "task_id": task_id.to_string(),
        }))
        .execute(&self.pg)
        .await?;

        let notice = SearchPositionResult {
            envelope: Some(envelope.clone()),
            component_id: component_id.to_string(),
            queries_issued: 0,
            results_found: 0,
            pages_fetched: 0,
            search_credits: 0,
            fetch_credits: 0,
            success: false,
            error_message: message.clone(),
        };
        self.streams
            .xadd_kind(streams::events::SEARCH, "search", &notice.encode_to_vec())
            .await?;

        progress::publish(
            &self.streams,
            EventSpec::new("search.failed", tender_id.to_string())
                .task(task_id.to_string())
                .run(run_id.to_string())
                .stage("SEARCHING")
                .level("error")
                .message(message),
        )
        .await?;

        Ok(())
    }
}

fn parse_uuid(field: &str, value: &str) -> AppResult<Uuid> {
    Uuid::parse_str(value)
        .map_err(|err| AppError::internal(format!("invalid {field} '{value}': {err}")))
}

fn is_terminal(status: &str) -> bool {
    matches!(status, "DONE" | "FAILED" | "CANCELLED")
}

/// Errors that mean "try again later" — the message stays in the PEL.
fn is_infra(err: &AppError) -> bool {
    matches!(
        err,
        AppError::Database(_) | AppError::Redis(_) | AppError::Io(_)
    )
}

fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    if value.trim().is_empty() {
        return None;
    }
    DateTime::parse_from_rfc3339(value.trim())
        .map(|parsed| parsed.with_timezone(&Utc))
        .ok()
}

fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_statuses() {
        assert!(is_terminal("DONE"));
        assert!(is_terminal("FAILED"));
        assert!(is_terminal("CANCELLED"));
        assert!(!is_terminal("SEARCHING"));
        assert!(!is_terminal("QUEUED"));
    }

    #[test]
    fn infra_errors_are_retryable() {
        assert!(is_infra(&AppError::Database(sqlx::Error::RowNotFound)));
        assert!(!is_infra(&AppError::Upstream("keenable 500".into())));
        assert!(!is_infra(&AppError::Validation("bad".into())));
    }

    #[test]
    fn timestamps_parse_rfc3339_only() {
        assert!(parse_timestamp("2026-01-15T10:00:00Z").is_some());
        assert!(parse_timestamp("2026-01-15T10:00:00+03:00").is_some());
        assert!(parse_timestamp("").is_none());
        assert!(parse_timestamp("not a date").is_none());
    }

    #[test]
    fn sha256_is_stable_hex() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn uuid_parsing_reports_the_field() {
        let err = parse_uuid("task_id", "nope").unwrap_err();
        assert!(err.to_string().contains("task_id"), "{err}");
    }
}
