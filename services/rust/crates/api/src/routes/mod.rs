//! REST API router.
//!
//! All endpoints live under `/api/v1`; `/healthz` and `/readyz` are exposed
//! at the root for orchestrators and load balancers.

use axum::routing::{get, patch, post};
use axum::Router;
use tower_http::trace::TraceLayer;

use crate::state::AppState;

pub mod activity;
pub mod assistant;
pub mod auth;
pub mod components;
pub mod documents;
pub mod health;
pub mod reports;
pub mod results;
pub mod tenders;
pub mod usage;

use axum::http::HeaderMap;
use uuid::Uuid;

use tender_common::error::{AppError, AppResult};

// `auth` below is this module's own auth routes; the session helpers live in
// the crate-level `crate::auth` module.
use crate::auth::SessionUser;

/// CSRF guard for every mutating endpoint.
pub fn require_csrf_for(user: &SessionUser, headers: &HeaderMap) -> AppResult<()> {
    crate::auth::require_csrf(user, headers)
}

/// Positions, requirements and documents may only be edited while the tender
/// is still being prepared. After the search starts, edits require a new run.
pub async fn ensure_tender_editable(state: &AppState, tender_id: Uuid) -> AppResult<()> {
    let status: Option<String> =
        sqlx::query_scalar("SELECT status FROM tenders WHERE id = $1 AND deleted_at IS NULL")
            .bind(tender_id)
            .fetch_optional(&state.pg)
            .await?;
    let status = status.ok_or_else(|| AppError::NotFound(format!("tender {tender_id}")))?;
    match status.as_str() {
        "CREATED"
        | "UPLOADING"
        | "DOCUMENT_PROCESSING"
        | "REQUIREMENT_EXTRACTION"
        | "REQUIREMENTS_REVIEW" => Ok(()),
        other => Err(AppError::Conflict(format!(
            "tender is in status {other}; editing is only allowed before the search starts"
        ))),
    }
}

/// Standard paginated response envelope.
#[derive(Debug, serde::Serialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

#[derive(Debug, serde::Deserialize)]
pub struct PageQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

impl PageQuery {
    /// Clamp to sane bounds: page >= 1, 1 <= page_size <= 100.
    pub fn bounds(&self) -> (i64, i64, i64) {
        let page = self.page.unwrap_or(1).clamp(1, 1_000_000);
        let page_size = self.page_size.unwrap_or(20).clamp(1, 100);
        (page, page_size, (page - 1) * page_size)
    }
}

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        // ---- auth ----
        .route("/auth/login", post(auth::login))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/me", get(auth::me))
        // ---- tenders ----
        .route("/tenders", get(tenders::list).post(tenders::create))
        .route(
            "/tenders/{id}",
            get(tenders::get)
                .patch(tenders::update)
                .delete(tenders::delete),
        )
        .route("/tenders/{id}/cancel", post(tenders::cancel))
        .route("/tenders/{id}/start-search", post(tenders::start_search))
        // ---- documents ----
        .route(
            "/tenders/{id}/documents",
            get(documents::list).post(documents::upload),
        )
        .route(
            "/documents/{id}",
            get(documents::get).delete(documents::delete),
        )
        // ---- components & requirements ----
        .route(
            "/tenders/{id}/components",
            get(components::list).post(components::create),
        )
        .route(
            "/components/{id}",
            patch(components::update).delete(components::remove),
        )
        .route(
            "/components/{id}/requirements",
            get(components::list_requirements).post(components::create_requirement),
        )
        .route(
            "/requirements/{id}",
            patch(components::update_requirement).delete(components::delete_requirement),
        )
        // ---- results ----
        .route("/tenders/{id}/results", get(results::tender_results))
        .route("/components/{id}/matches", get(results::component_matches))
        .route("/tenders/{id}/runs", get(results::runs))
        // ---- reports ----
        .route("/tenders/{id}/reports", get(reports::list))
        .route("/reports/{id}/download", get(reports::download))
        // ---- AI intake ----
        .route("/assistant/parse", post(assistant::parse))
        .route("/assistant/{id}", get(assistant::get))
        .route("/assistant/{id}/apply", post(assistant::apply))
        // ---- activity & usage ----
        .route("/tenders/{id}/activity", get(activity::tender_activity))
        .route("/activity", get(activity::recent))
        .route("/usage/summary", get(usage::summary))
        // ---- realtime ----
        .route("/ws/tenders/{id}", get(crate::ws::tender_progress));

    Router::new()
        .nest("/api/v1", api)
        .merge(health::router())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
