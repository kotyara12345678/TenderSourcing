//! Activity log endpoints.

use axum::extract::{Path, Query, State};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use uuid::Uuid;

use tender_common::error::AppResult;

use crate::auth::SessionUser;
use crate::state::AppState;

#[derive(Debug, serde::Serialize, sqlx::FromRow)]
pub struct ActivityView {
    pub id: Uuid,
    pub tender_id: Option<Uuid>,
    pub event_type: String,
    pub message: String,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct LimitQuery {
    pub limit: Option<i64>,
}

fn limit(query: &LimitQuery) -> i64 {
    query.limit.unwrap_or(50).clamp(1, 200)
}

pub async fn tender_activity(
    State(state): State<AppState>,
    _user: SessionUser,
    Path(tender_id): Path<Uuid>,
    Query(query): Query<LimitQuery>,
) -> AppResult<Json<Vec<ActivityView>>> {
    let items: Vec<ActivityView> = sqlx::query_as(
        "SELECT id, tender_id, event_type, message, metadata, created_at
         FROM activity_logs WHERE tender_id = $1
         ORDER BY created_at DESC, id DESC LIMIT $2",
    )
    .bind(tender_id)
    .bind(limit(&query))
    .fetch_all(&state.pg)
    .await?;
    Ok(Json(items))
}

pub async fn recent(
    State(state): State<AppState>,
    _user: SessionUser,
    Query(query): Query<LimitQuery>,
) -> AppResult<Json<Vec<ActivityView>>> {
    let items: Vec<ActivityView> = sqlx::query_as(
        "SELECT id, tender_id, event_type, message, metadata, created_at
         FROM activity_logs ORDER BY created_at DESC, id DESC LIMIT $1",
    )
    .bind(limit(&query))
    .fetch_all(&state.pg)
    .await?;
    Ok(Json(items))
}
