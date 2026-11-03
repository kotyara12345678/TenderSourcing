//! Document endpoints: upload (multipart), list, detail, delete.

use axum::extract::{Multipart, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use chrono::{DateTime, Utc};
use prost::Message;
use uuid::Uuid;

use tender_common::error::{AppError, AppResult};
use tender_common::streams;

use super::{ensure_tender_editable, require_csrf_for};
use crate::auth::SessionUser;
use crate::state::AppState;
use crate::storage;
use crate::tender::common::v1::JobEnvelope;
use crate::tender::documents::v1::ExtractDocumentJob;

#[derive(Debug, serde::Serialize, sqlx::FromRow)]
pub struct DocumentView {
    pub id: Uuid,
    pub tender_id: Uuid,
    pub original_filename: String,
    pub mime_type: String,
    pub size_bytes: i64,
    pub status: String,
    pub ai_status: String,
    pub error_message: Option<String>,
    pub page_count: Option<i32>,
    pub ocr_used: bool,
    pub uploaded_at: DateTime<Utc>,
    pub processed_at: Option<DateTime<Utc>>,
}

const DOCUMENT_COLUMNS: &str = "id, tender_id, original_filename, mime_type, size_bytes, status,
    ai_status, error_message, page_count, ocr_used, uploaded_at, processed_at";

async fn load_document(state: &AppState, document_id: Uuid) -> AppResult<DocumentView> {
    sqlx::query_as(&format!(
        "SELECT {DOCUMENT_COLUMNS} FROM documents WHERE id = $1 AND deleted_at IS NULL"
    ))
    .bind(document_id)
    .fetch_optional(&state.pg)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("document {document_id}")))
}

pub async fn list(
    State(state): State<AppState>,
    _user: SessionUser,
    Path(tender_id): Path<Uuid>,
) -> AppResult<Json<Vec<DocumentView>>> {
    let items: Vec<DocumentView> = sqlx::query_as(&format!(
        "SELECT {DOCUMENT_COLUMNS} FROM documents
         WHERE tender_id = $1 AND deleted_at IS NULL
         ORDER BY uploaded_at DESC, id"
    ))
    .bind(tender_id)
    .fetch_all(&state.pg)
    .await?;
    Ok(Json(items))
}

pub async fn upload(
    State(state): State<AppState>,
    user: SessionUser,
    headers: HeaderMap,
    Path(tender_id): Path<Uuid>,
    mut multipart: Multipart,
) -> AppResult<(StatusCode, Json<DocumentView>)> {
    require_csrf_for(&user, &headers)?;
    ensure_tender_editable(&state, tender_id).await?;

    let mut filename = String::new();
    let mut declared_mime = String::new();
    let mut bytes: Vec<u8> = Vec::new();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|err| AppError::BadRequest(format!("invalid multipart body: {err}")))?
    {
        if field.name() != Some("file") {
            continue;
        }
        filename = field.file_name().unwrap_or("upload").to_string();
        declared_mime = field
            .content_type()
            .unwrap_or("application/octet-stream")
            .to_string();
        let data = field
            .bytes()
            .await
            .map_err(|err| AppError::BadRequest(format!("cannot read uploaded file: {err}")))?;
        if data.len() as u64 > state.cfg.max_file_size {
            return Err(AppError::Validation(format!(
                "file exceeds the maximum upload size of {} bytes",
                state.cfg.max_file_size
            )));
        }
        bytes = data.to_vec();
        break;
    }

    if bytes.is_empty() {
        return Err(AppError::Validation(
            "multipart field 'file' is required".into(),
        ));
    }
    storage::validate_upload(state.cfg.max_file_size, &filename, &bytes)?;
    let sha256 = storage::sha256_hex(&bytes);

    // Identical uploads within one tender are deduplicated by (tender, sha256).
    let existing: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM documents
         WHERE tender_id = $1 AND sha256 = $2 AND deleted_at IS NULL",
    )
    .bind(tender_id)
    .bind(&sha256)
    .fetch_optional(&state.pg)
    .await?;
    if let Some(document_id) = existing {
        return Ok((
            StatusCode::OK,
            Json(load_document(&state, document_id).await?),
        ));
    }

    let stored = storage::store_upload(&state.cfg, tender_id, &filename, &bytes).await?;
    let document_id: Uuid = sqlx::query_scalar(
        "INSERT INTO documents
            (tender_id, original_filename, storage_key, mime_type, size_bytes, sha256, status)
         VALUES ($1, $2, $3, $4, $5, $6, 'QUEUED')
         RETURNING id",
    )
    .bind(tender_id)
    .bind(filename.chars().take(400).collect::<String>())
    .bind(&stored.key)
    .bind(declared_mime.chars().take(200).collect::<String>())
    .bind(stored.size.min(i64::MAX as u64) as i64)
    .bind(&stored.sha256)
    .fetch_one(&state.pg)
    .await?;

    let job = ExtractDocumentJob {
        envelope: Some(JobEnvelope {
            task_id: document_id.to_string(),
            tender_id: tender_id.to_string(),
            run_id: String::new(),
            created_at_unix_ms: Utc::now().timestamp_millis(),
            attempt: 1,
            metadata: Default::default(),
        }),
        document_id: document_id.to_string(),
        storage_key: stored.key.clone(),
        original_filename: filename.clone(),
        mime_type: declared_mime.clone(),
        size_bytes: stored.size.min(i64::MAX as u64) as i64,
        sha256: stored.sha256.clone(),
        run_ocr: state.cfg.ocr_provider != "none",
    };
    state
        .streams
        .xadd(streams::jobs::DOCUMENTS, &job.encode_to_vec())
        .await?;

    // The orchestrator owns the FSM; ask it to move the tender into the
    // document-processing stage (idempotent for later uploads).
    let control = JobEnvelope {
        task_id: Uuid::new_v4().to_string(),
        tender_id: tender_id.to_string(),
        run_id: String::new(),
        created_at_unix_ms: Utc::now().timestamp_millis(),
        attempt: 1,
        metadata: std::collections::HashMap::from([(
            "action".to_string(),
            "documents_uploaded".to_string(),
        )]),
    };
    state
        .streams
        .xadd(streams::jobs::CONTROL, &control.encode_to_vec())
        .await?;

    sqlx::query(
        "INSERT INTO activity_logs (tender_id, user_id, event_type, message, metadata)
         VALUES ($1, $2, 'document.uploaded', $3, $4)",
    )
    .bind(tender_id)
    .bind(user.id)
    .bind(format!("document '{filename}' uploaded"))
    .bind(serde_json::json!({ "document_id": document_id.to_string(), "size_bytes": stored.size }))
    .execute(&state.pg)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(load_document(&state, document_id).await?),
    ))
}

#[derive(Debug, serde::Serialize, sqlx::FromRow)]
pub struct PagePreview {
    pub page_number: i32,
    pub content: String,
    pub ocr_used: bool,
}

#[derive(Debug, serde::Serialize)]
pub struct DocumentDetail {
    #[serde(flatten)]
    pub document: DocumentView,
    pub pages: Vec<PagePreview>,
}

pub async fn get(
    State(state): State<AppState>,
    _user: SessionUser,
    Path(document_id): Path<Uuid>,
) -> AppResult<Json<DocumentDetail>> {
    let document = load_document(&state, document_id).await?;
    let pages: Vec<PagePreview> = sqlx::query_as(
        "SELECT page_number, content, ocr_used FROM document_pages
         WHERE document_id = $1 ORDER BY page_number LIMIT 3",
    )
    .bind(document_id)
    .fetch_all(&state.pg)
    .await?
    .into_iter()
    .map(|mut page: PagePreview| {
        page.content = page.content.chars().take(2000).collect();
        page
    })
    .collect();

    Ok(Json(DocumentDetail { document, pages }))
}

pub async fn delete(
    State(state): State<AppState>,
    user: SessionUser,
    headers: HeaderMap,
    Path(document_id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    require_csrf_for(&user, &headers)?;
    let document = load_document(&state, document_id).await?;
    if matches!(document.status.as_str(), "QUEUED" | "PROCESSING") {
        return Err(AppError::Conflict(
            "the document is being processed; try again in a moment".into(),
        ));
    }
    ensure_tender_editable(&state, document.tender_id).await?;

    let storage_key: String = sqlx::query_scalar("SELECT storage_key FROM documents WHERE id = $1")
        .bind(document_id)
        .fetch_one(&state.pg)
        .await?;

    sqlx::query("UPDATE documents SET deleted_at = now() WHERE id = $1")
        .bind(document_id)
        .execute(&state.pg)
        .await?;

    if let Ok(path) =
        storage::resolve_path(std::path::Path::new(&state.cfg.storage_path), &storage_key)
    {
        let _ = tokio::fs::remove_file(path).await;
    }

    sqlx::query(
        "INSERT INTO activity_logs (tender_id, user_id, event_type, message, metadata)
         VALUES ($1, $2, 'document.deleted', $3, $4)",
    )
    .bind(document.tender_id)
    .bind(user.id)
    .bind(format!("document '{}' deleted", document.original_filename))
    .bind(serde_json::json!({ "document_id": document_id.to_string() }))
    .execute(&state.pg)
    .await?;

    Ok(Json(serde_json::json!({ "ok": true })))
}
