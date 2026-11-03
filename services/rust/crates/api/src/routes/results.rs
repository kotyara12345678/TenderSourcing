//! Results: ranked matches per position, per-requirement details, runs.

use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use tender_common::error::AppResult;

use super::tenders::RunSummary;
use crate::auth::SessionUser;
use crate::state::AppState;

#[derive(Debug, serde::Serialize, sqlx::FromRow)]
pub struct ComponentResultHeader {
    pub id: Uuid,
    pub position_no: i32,
    pub name: String,
    pub quantity: f64,
    pub unit: String,
    pub status: String,
}

#[derive(Debug, serde::Serialize, sqlx::FromRow)]
pub struct MatchView {
    pub component_id: Uuid,
    pub match_id: Uuid,
    pub verdict: String,
    pub eligible: bool,
    pub technical_score: f64,
    pub rank: Option<i32>,
    pub total_score: Option<f64>,
    pub explanation: String,
    pub product_id: Uuid,
    pub product_name: String,
    pub manufacturer: String,
    pub model: String,
    pub price: Option<f64>,
    pub currency: Option<String>,
    pub lead_time_days: Option<i32>,
    pub canonical_url: String,
    pub supplier_name: Option<String>,
    pub supplier_url: Option<String>,
}

impl MatchView {
    fn component_id(&self) -> Uuid {
        self.component_id
    }
}

#[derive(Debug, serde::Serialize)]
pub struct ComponentResults {
    #[serde(flatten)]
    pub component: ComponentResultHeader,
    pub matches: Vec<MatchView>,
}

#[derive(Debug, serde::Serialize, sqlx::FromRow)]
pub struct MatchDetailView {
    pub match_id: Uuid,
    pub requirement_name: String,
    pub status: String,
    pub expected: String,
    pub actual: String,
    pub unit: String,
    pub explanation: String,
}

#[derive(Debug, serde::Serialize)]
pub struct ComponentMatchBundle {
    pub component_id: Uuid,
    pub matches: Vec<MatchView>,
    pub details: Vec<MatchDetailView>,
}

const MATCH_SELECT: &str = "SELECT m.component_id, m.id AS match_id, m.verdict, m.eligible,
        m.technical_score::float8 AS technical_score, m.rank,
        m.total_score::float8 AS total_score, m.explanation,
        p.id AS product_id, p.name AS product_name, p.manufacturer, p.model,
        p.price::float8 AS price, p.currency::text AS currency, p.lead_time_days,
        p.canonical_url, sup.name AS supplier_name, sup.url AS supplier_url
    FROM matches m
    JOIN products p ON p.id = m.product_id
    LEFT JOIN suppliers sup ON sup.id = p.supplier_id";

pub async fn tender_results(
    State(state): State<AppState>,
    _user: SessionUser,
    Path(tender_id): Path<Uuid>,
) -> AppResult<Json<Vec<ComponentResults>>> {
    let components: Vec<ComponentResultHeader> = sqlx::query_as(
        "SELECT id, position_no, name, quantity::float8 AS quantity, unit, status
         FROM components WHERE tender_id = $1 ORDER BY position_no, id",
    )
    .bind(tender_id)
    .fetch_all(&state.pg)
    .await?;

    let matches: Vec<MatchView> = sqlx::query_as(&format!(
        "{MATCH_SELECT}
         WHERE m.component_id = ANY($1)
         ORDER BY m.component_id, m.rank NULLS LAST, m.total_score DESC NULLS LAST, p.id"
    ))
    .bind(
        components
            .iter()
            .map(|component| component.id)
            .collect::<Vec<Uuid>>(),
    )
    .fetch_all(&state.pg)
    .await?;

    let results = components
        .into_iter()
        .map(|component| {
            let component_matches: Vec<MatchView> = matches
                .iter()
                .filter(|view| view.component_id() == component.id)
                .map(clone_match)
                .collect();
            ComponentResults {
                component,
                matches: component_matches,
            }
        })
        .collect();

    Ok(Json(results))
}

pub async fn component_matches(
    State(state): State<AppState>,
    _user: SessionUser,
    Path(component_id): Path<Uuid>,
) -> AppResult<Json<ComponentMatchBundle>> {
    let matches: Vec<MatchView> = sqlx::query_as(&format!(
        "{MATCH_SELECT}
         WHERE m.component_id = $1
         ORDER BY m.rank NULLS LAST, m.total_score DESC NULLS LAST, p.id"
    ))
    .bind(component_id)
    .fetch_all(&state.pg)
    .await?;

    let details: Vec<MatchDetailView> = sqlx::query_as(
        "SELECT d.match_id, d.requirement_name, d.status, d.expected, d.actual, d.unit, d.explanation
         FROM match_details d
         JOIN matches m ON m.id = d.match_id
         WHERE m.component_id = $1
         ORDER BY d.match_id, d.requirement_name",
    )
    .bind(component_id)
    .fetch_all(&state.pg)
    .await?;

    Ok(Json(ComponentMatchBundle {
        component_id,
        matches,
        details,
    }))
}

pub async fn runs(
    State(state): State<AppState>,
    _user: SessionUser,
    Path(tender_id): Path<Uuid>,
) -> AppResult<Json<Vec<RunSummary>>> {
    let items: Vec<RunSummary> = sqlx::query_as(
        "SELECT id, status, components_total, components_done, components_failed,
                queries_issued, results_found, pages_fetched, started_at, finished_at, created_at
         FROM search_runs WHERE tender_id = $1 ORDER BY created_at DESC",
    )
    .bind(tender_id)
    .fetch_all(&state.pg)
    .await?;
    Ok(Json(items))
}

fn clone_match(view: &MatchView) -> MatchView {
    MatchView {
        component_id: view.component_id,
        match_id: view.match_id,
        verdict: view.verdict.clone(),
        eligible: view.eligible,
        technical_score: view.technical_score,
        rank: view.rank,
        total_score: view.total_score,
        explanation: view.explanation.clone(),
        product_id: view.product_id,
        product_name: view.product_name.clone(),
        manufacturer: view.manufacturer.clone(),
        model: view.model.clone(),
        price: view.price,
        currency: view.currency.clone(),
        lead_time_days: view.lead_time_days,
        canonical_url: view.canonical_url.clone(),
        supplier_name: view.supplier_name.clone(),
        supplier_url: view.supplier_url.clone(),
    }
}
