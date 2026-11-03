//! Report endpoints: list and download generated files.

use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use tender_common::error::{AppError, AppResult};

use crate::auth::SessionUser;
use crate::state::AppState;
use crate::storage;

#[derive(Debug, serde::Serialize, sqlx::FromRow)]
pub struct ReportView {
    pub id: Uuid,
    pub tender_id: Uuid,
    pub run_id: Option<Uuid>,
    pub format: String,
    pub size_bytes: i64,
    pub status: String,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub ready_at: Option<DateTime<Utc>>,
}

pub async fn list(
    State(state): State<AppState>,
    _user: SessionUser,
    Path(tender_id): Path<Uuid>,
) -> AppResult<Json<Vec<ReportView>>> {
    let items: Vec<ReportView> = sqlx::query_as(
        "SELECT id, tender_id, run_id, format, size_bytes, status, error_message, created_at, ready_at
         FROM reports WHERE tender_id = $1 ORDER BY created_at DESC",
    )
    .bind(tender_id)
    .fetch_all(&state.pg)
    .await?;
    Ok(Json(items))
}

pub async fn download(
    State(state): State<AppState>,
    _user: SessionUser,
    Path(report_id): Path<Uuid>,
) -> AppResult<Response> {
    let row: Option<(String, String, String)> =
        sqlx::query_as("SELECT storage_key, format, status FROM reports WHERE id = $1")
            .bind(report_id)
            .fetch_optional(&state.pg)
            .await?;
    let (storage_key, format, status) =
        row.ok_or_else(|| AppError::NotFound(format!("report {report_id}")))?;
    if status != "READY" {
        return Err(AppError::Conflict(format!(
            "report is not ready yet (status {status})"
        )));
    }

    let bytes = storage::read_file(&state.cfg, &storage_key).await?;
    let content_type = match format.as_str() {
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    };
    let disposition = format!("attachment; filename=\"report-{report_id}.{format}\"");

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        content_type
            .parse()
            .map_err(|_| AppError::internal("content type"))?,
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        disposition
            .parse()
            .map_err(|_| AppError::internal("content disposition"))?,
    );
    headers.insert(
        header::CACHE_CONTROL,
        "private, no-store"
            .parse()
            .map_err(|_| AppError::internal("cache control"))?,
    );

    Ok((StatusCode::OK, headers, bytes).into_response())
}
