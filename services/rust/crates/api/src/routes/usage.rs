//! External API usage accounting (Keenable, LLM, OCR).

use axum::extract::State;
use axum::Json;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use tender_common::error::AppResult;

use crate::auth::SessionUser;
use crate::state::AppState;

#[derive(Debug, serde::Serialize, sqlx::FromRow)]
pub struct UsageByService {
    pub service: String,
    pub calls: i64,
    pub credits: f64,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub cost_usd: f64,
}

#[derive(Debug, serde::Serialize, sqlx::FromRow)]
pub struct UsageByTender {
    pub tender_id: Uuid,
    pub tender_name: String,
    pub calls: i64,
    pub credits: f64,
    pub cost_usd: f64,
}

#[derive(Debug, serde::Serialize, sqlx::FromRow)]
pub struct RecentUsage {
    pub id: Uuid,
    pub service: String,
    pub sku: String,
    pub tender_id: Option<Uuid>,
    pub quantity: i64,
    pub credits: f64,
    pub cost_usd: f64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, serde::Serialize, sqlx::FromRow)]
pub struct UsageTotals {
    pub calls: i64,
    pub credits: f64,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub cost_usd: f64,
}

#[derive(Debug, serde::Serialize)]
pub struct UsageSummary {
    pub totals: UsageTotals,
    pub by_service: Vec<UsageByService>,
    pub by_tender: Vec<UsageByTender>,
    pub recent: Vec<RecentUsage>,
}

pub async fn summary(
    State(state): State<AppState>,
    _user: SessionUser,
) -> AppResult<Json<UsageSummary>> {
    let totals: UsageTotals = sqlx::query_as(
        "SELECT COUNT(*) AS calls,
                COALESCE(SUM(credits), 0)::float8 AS credits,
                COALESCE(SUM(tokens_in), 0)::bigint AS tokens_in,
                COALESCE(SUM(tokens_out), 0)::bigint AS tokens_out,
                COALESCE(SUM(cost_usd), 0)::float8 AS cost_usd
         FROM api_usage",
    )
    .fetch_one(&state.pg)
    .await?;

    let by_service: Vec<UsageByService> = sqlx::query_as(
        "SELECT service, COUNT(*) AS calls,
                COALESCE(SUM(credits), 0)::float8 AS credits,
                COALESCE(SUM(tokens_in), 0)::bigint AS tokens_in,
                COALESCE(SUM(tokens_out), 0)::bigint AS tokens_out,
                COALESCE(SUM(cost_usd), 0)::float8 AS cost_usd
         FROM api_usage GROUP BY service ORDER BY service",
    )
    .fetch_all(&state.pg)
    .await?;

    let by_tender: Vec<UsageByTender> = sqlx::query_as(
        "SELECT u.tender_id, COALESCE(t.name, '(deleted tender)') AS tender_name,
                COUNT(*) AS calls,
                COALESCE(SUM(u.credits), 0)::float8 AS credits,
                COALESCE(SUM(u.cost_usd), 0)::float8 AS cost_usd
         FROM api_usage u
         LEFT JOIN tenders t ON t.id = u.tender_id
         WHERE u.tender_id IS NOT NULL
         GROUP BY u.tender_id, t.name
         ORDER BY credits DESC, u.tender_id
         LIMIT 20",
    )
    .fetch_all(&state.pg)
    .await?;

    let recent: Vec<RecentUsage> = sqlx::query_as(
        "SELECT id, service, sku, tender_id, quantity, credits::float8 AS credits,
                cost_usd::float8 AS cost_usd, created_at
         FROM api_usage ORDER BY created_at DESC, id DESC LIMIT 50",
    )
    .fetch_all(&state.pg)
    .await?;

    Ok(Json(UsageSummary {
        totals,
        by_service,
        by_tender,
        recent,
    }))
}
