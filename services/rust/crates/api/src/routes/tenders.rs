//! Tender endpoints.

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use chrono::{DateTime, Utc};
use prost::Message;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use tender_common::error::{AppError, AppResult};
use tender_common::streams;

use super::{require_csrf_for, Page, PageQuery};
use crate::auth::SessionUser;
use crate::state::AppState;
use crate::tender::common::v1::JobEnvelope;

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct TenderSummary {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub status: String,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub components_count: i64,
}

#[derive(Debug, Serialize)]
pub struct TenderDetail {
    #[serde(flatten)]
    pub tender: TenderSummary,
    pub documents_count: i64,
    pub reports_count: i64,
    pub latest_run: Option<RunSummary>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct RunSummary {
    pub id: Uuid,
    pub status: String,
    pub components_total: i32,
    pub components_done: i32,
    pub components_failed: i32,
    pub queries_issued: i32,
    pub results_found: i32,
    pub pages_fetched: i32,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    pub status: Option<String>,
    pub q: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateRequest {
    pub name: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateRequest {
    pub name: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StartSearchResponse {
    pub run_id: Uuid,
    pub positions: i64,
}

fn clean_filter(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub async fn list(
    State(state): State<AppState>,
    _user: SessionUser,
    Query(query): Query<ListQuery>,
) -> AppResult<Json<Page<TenderSummary>>> {
    let page_query = PageQuery {
        page: query.page,
        page_size: query.page_size,
    };
    let (page, page_size, offset) = page_query.bounds();
    let status = clean_filter(query.status).map(|value| value.to_uppercase());
    let search = clean_filter(query.q);

    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM tenders
         WHERE deleted_at IS NULL
           AND ($1::text IS NULL OR status = $1)
           AND ($2::text IS NULL OR name ILIKE '%' || $2 || '%')",
    )
    .bind(&status)
    .bind(&search)
    .fetch_one(&state.pg)
    .await?;

    let items: Vec<TenderSummary> = sqlx::query_as(
        "SELECT t.id, t.name, t.description, t.status, t.error_message,
                t.created_at, t.updated_at,
                (SELECT COUNT(*) FROM components c WHERE c.tender_id = t.id) AS components_count
         FROM tenders t
         WHERE t.deleted_at IS NULL
           AND ($1::text IS NULL OR t.status = $1)
           AND ($2::text IS NULL OR t.name ILIKE '%' || $2 || '%')
         ORDER BY t.created_at DESC, t.id
         LIMIT $3 OFFSET $4",
    )
    .bind(&status)
    .bind(&search)
    .bind(page_size)
    .bind(offset)
    .fetch_all(&state.pg)
    .await?;

    Ok(Json(Page {
        items,
        total,
        page,
        page_size,
    }))
}

async fn load_summary(state: &AppState, tender_id: Uuid) -> AppResult<TenderSummary> {
    sqlx::query_as(
        "SELECT t.id, t.name, t.description, t.status, t.error_message,
                t.created_at, t.updated_at,
                (SELECT COUNT(*) FROM components c WHERE c.tender_id = t.id) AS components_count
         FROM tenders t WHERE t.id = $1 AND t.deleted_at IS NULL",
    )
    .bind(tender_id)
    .fetch_optional(&state.pg)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("tender {tender_id}")))
}

pub async fn create(
    State(state): State<AppState>,
    user: SessionUser,
    headers: HeaderMap,
    Json(body): Json<CreateRequest>,
) -> AppResult<(StatusCode, Json<TenderSummary>)> {
    require_csrf_for(&user, &headers)?;
    let name = body.name.trim().to_string();
    if name.is_empty() || name.chars().count() > 200 {
        return Err(AppError::Validation(
            "name must be between 1 and 200 characters".into(),
        ));
    }
    let description: String = body.description.trim().chars().take(4000).collect();

    let tender_id: Uuid = sqlx::query_scalar(
        "INSERT INTO tenders (name, description, created_by) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(&name)
    .bind(&description)
    .bind(user.id)
    .fetch_one(&state.pg)
    .await?;

    sqlx::query(
        "INSERT INTO activity_logs (tender_id, user_id, event_type, message, metadata)
         VALUES ($1, $2, 'tender.created', $3, '{}')",
    )
    .bind(tender_id)
    .bind(user.id)
    .bind(format!("tender '{name}' created"))
    .execute(&state.pg)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(load_summary(&state, tender_id).await?),
    ))
}

pub async fn get(
    State(state): State<AppState>,
    _user: SessionUser,
    Path(tender_id): Path<Uuid>,
) -> AppResult<Json<TenderDetail>> {
    let tender = load_summary(&state, tender_id).await?;
    let documents_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM documents WHERE tender_id = $1 AND deleted_at IS NULL",
    )
    .bind(tender_id)
    .fetch_one(&state.pg)
    .await?;
    let reports_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM reports WHERE tender_id = $1")
            .bind(tender_id)
            .fetch_one(&state.pg)
            .await?;
    let latest_run: Option<RunSummary> = sqlx::query_as(
        "SELECT id, status, components_total, components_done, components_failed,
                queries_issued, results_found, pages_fetched, started_at, finished_at, created_at
         FROM search_runs WHERE tender_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(tender_id)
    .fetch_optional(&state.pg)
    .await?;

    Ok(Json(TenderDetail {
        tender,
        documents_count,
        reports_count,
        latest_run,
    }))
}

pub async fn update(
    State(state): State<AppState>,
    user: SessionUser,
    headers: HeaderMap,
    Path(tender_id): Path<Uuid>,
    Json(body): Json<UpdateRequest>,
) -> AppResult<Json<TenderSummary>> {
    require_csrf_for(&user, &headers)?;
    let current = load_summary(&state, tender_id).await?;
    if matches!(
        current.status.as_str(),
        "COMPLETED" | "COMPLETED_WITH_ERRORS" | "FAILED" | "CANCELLED"
    ) {
        return Err(AppError::Conflict(
            "a finished tender cannot be edited".into(),
        ));
    }
    let name = body
        .name
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if let Some(name) = &name {
        if name.chars().count() > 200 {
            return Err(AppError::Validation(
                "name must be at most 200 characters".into(),
            ));
        }
    }
    let description: Option<String> = body
        .description
        .map(|value| value.trim().chars().take(4000).collect());

    sqlx::query("UPDATE tenders SET name = COALESCE($2, name), description = COALESCE($3, description) WHERE id = $1")
        .bind(tender_id)
        .bind(name)
        .bind(description)
        .execute(&state.pg)
        .await?;
    Ok(Json(load_summary(&state, tender_id).await?))
}

pub async fn delete(
    State(state): State<AppState>,
    user: SessionUser,
    headers: HeaderMap,
    Path(tender_id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    require_csrf_for(&user, &headers)?;
    let current = load_summary(&state, tender_id).await?;
    let finished = matches!(
        current.status.as_str(),
        "COMPLETED" | "COMPLETED_WITH_ERRORS" | "FAILED" | "CANCELLED"
    );
    if !finished && !matches!(current.status.as_str(), "CREATED" | "REQUIREMENTS_REVIEW") {
        return Err(AppError::Conflict(
            "cancel the tender before deleting it".into(),
        ));
    }
    sqlx::query("UPDATE tenders SET deleted_at = now() WHERE id = $1")
        .bind(tender_id)
        .execute(&state.pg)
        .await?;
    sqlx::query(
        "INSERT INTO activity_logs (tender_id, user_id, event_type, message, metadata)
         VALUES ($1, $2, 'tender.deleted', 'tender deleted', '{}')",
    )
    .bind(tender_id)
    .bind(user.id)
    .execute(&state.pg)
    .await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn cancel(
    State(state): State<AppState>,
    user: SessionUser,
    headers: HeaderMap,
    Path(tender_id): Path<Uuid>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    require_csrf_for(&user, &headers)?;
    let current = load_summary(&state, tender_id).await?;
    if matches!(
        current.status.as_str(),
        "COMPLETED" | "COMPLETED_WITH_ERRORS" | "FAILED" | "CANCELLED"
    ) {
        return Err(AppError::Conflict("the tender is already finished".into()));
    }

    let envelope = JobEnvelope {
        task_id: Uuid::new_v4().to_string(),
        tender_id: tender_id.to_string(),
        run_id: String::new(),
        created_at_unix_ms: Utc::now().timestamp_millis(),
        attempt: 1,
        metadata: HashMap::from([("action".to_string(), "cancel".to_string())]),
    };
    state
        .streams
        .xadd(streams::jobs::CONTROL, &envelope.encode_to_vec())
        .await?;

    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "queued": true })),
    ))
}

pub async fn start_search(
    State(state): State<AppState>,
    user: SessionUser,
    headers: HeaderMap,
    Path(tender_id): Path<Uuid>,
) -> AppResult<Json<StartSearchResponse>> {
    require_csrf_for(&user, &headers)?;
    let current = load_summary(&state, tender_id).await?;
    if !matches!(
        current.status.as_str(),
        "CREATED" | "REQUIREMENTS_REVIEW" | "SEARCH_QUEUED"
    ) {
        return Err(AppError::Conflict(format!(
            "search can start from REQUIREMENTS_REVIEW, the tender is in {}",
            current.status
        )));
    }

    let positions: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM components WHERE tender_id = $1 AND status <> 'SKIPPED'",
    )
    .bind(tender_id)
    .fetch_one(&state.pg)
    .await?;
    if positions == 0 {
        return Err(AppError::Conflict(
            "add at least one position before starting the search".into(),
        ));
    }
    if positions as u64 > u64::from(state.cfg.max_tender_components) {
        return Err(AppError::Validation(format!(
            "tender has {positions} positions; the limit is {}",
            state.cfg.max_tender_components
        )));
    }

    let run_id: Uuid = sqlx::query_scalar(
        "INSERT INTO search_runs (tender_id, triggered_by, components_total)
         VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(tender_id)
    .bind(user.id)
    .bind(positions.min(i64::from(i32::MAX)) as i32)
    .fetch_one(&state.pg)
    .await?;

    sqlx::query(
        "INSERT INTO search_tasks (run_id, component_id)
         SELECT $1, id FROM components WHERE tender_id = $2 AND status <> 'SKIPPED'
         ON CONFLICT (run_id, component_id) DO NOTHING",
    )
    .bind(run_id)
    .bind(tender_id)
    .execute(&state.pg)
    .await?;

    sqlx::query(
        "UPDATE components SET status = 'QUEUED'
         WHERE tender_id = $1 AND status NOT IN ('SKIPPED','MATCHED')",
    )
    .bind(tender_id)
    .execute(&state.pg)
    .await?;

    let envelope = JobEnvelope {
        task_id: Uuid::new_v4().to_string(),
        tender_id: tender_id.to_string(),
        run_id: run_id.to_string(),
        created_at_unix_ms: Utc::now().timestamp_millis(),
        attempt: 1,
        metadata: HashMap::from([("action".to_string(), "start_run".to_string())]),
    };
    state
        .streams
        .xadd(streams::jobs::CONTROL, &envelope.encode_to_vec())
        .await?;

    sqlx::query(
        "INSERT INTO activity_logs (tender_id, user_id, event_type, message, metadata)
         VALUES ($1, $2, 'search.requested', $3, $4)",
    )
    .bind(tender_id)
    .bind(user.id)
    .bind(format!("search run requested for {positions} position(s)"))
    .bind(serde_json::json!({ "run_id": run_id.to_string() }))
    .execute(&state.pg)
    .await?;

    Ok(Json(StartSearchResponse { run_id, positions }))
}
