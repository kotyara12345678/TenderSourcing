//! Positions (components) and their technical requirements.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use uuid::Uuid;

use tender_common::error::{AppError, AppResult};

use super::{ensure_tender_editable, require_csrf_for};
use crate::auth::SessionUser;
use crate::state::AppState;

const COMPONENT_COLUMNS: &str = "c.id, c.position_no, c.name, c.quantity::float8 AS quantity, c.unit,
    c.category, c.status, c.source, c.error_message,
    (SELECT COUNT(*) FROM requirements r WHERE r.component_id = c.id) AS requirements_count,
    (SELECT COUNT(*) FROM requirements r WHERE r.component_id = c.id AND r.mandatory) AS mandatory_count";

const REQUIREMENT_COLUMNS: &str = "id, component_id, name, value_text,
    value_num::float8 AS value_num, unit, data_type, mandatory,
    range_min::float8 AS range_min, range_max::float8 AS range_max,
    tolerance::float8 AS tolerance, allowed_values, source, confidence::float8 AS confidence";

const DATA_TYPES: [&str; 9] = [
    "STRING",
    "NUMBER",
    "RANGE",
    "BOOLEAN",
    "ENUM",
    "STANDARD",
    "MATERIAL",
    "MODEL",
    "DIMENSION",
];

#[derive(Debug, serde::Serialize, sqlx::FromRow)]
pub struct ComponentView {
    pub id: Uuid,
    pub position_no: i32,
    pub name: String,
    pub quantity: f64,
    pub unit: String,
    pub category: String,
    pub status: String,
    pub source: String,
    pub error_message: Option<String>,
    pub requirements_count: i64,
    pub mandatory_count: i64,
}

#[derive(Debug, serde::Serialize, sqlx::FromRow)]
pub struct RequirementView {
    pub id: Uuid,
    pub component_id: Uuid,
    pub name: String,
    pub value_text: String,
    pub value_num: Option<f64>,
    pub unit: String,
    pub data_type: String,
    pub mandatory: bool,
    pub range_min: Option<f64>,
    pub range_max: Option<f64>,
    pub tolerance: Option<f64>,
    pub allowed_values: serde_json::Value,
    pub source: String,
    pub confidence: f64,
}

#[derive(Debug, serde::Deserialize)]
pub struct ComponentInput {
    pub name: String,
    pub quantity: Option<f64>,
    pub unit: Option<String>,
    pub category: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct RequirementInput {
    pub name: String,
    #[serde(default)]
    pub value_text: String,
    pub value_num: Option<f64>,
    #[serde(default)]
    pub unit: String,
    #[serde(default = "default_data_type")]
    pub data_type: String,
    #[serde(default)]
    pub mandatory: bool,
    pub range_min: Option<f64>,
    pub range_max: Option<f64>,
    pub tolerance: Option<f64>,
    #[serde(default)]
    pub allowed_values: Vec<String>,
}

fn default_data_type() -> String {
    "STRING".to_string()
}

async fn tender_of_component(state: &AppState, component_id: Uuid) -> AppResult<Uuid> {
    let tender_id: Option<Uuid> =
        sqlx::query_scalar("SELECT tender_id FROM components WHERE id = $1")
            .bind(component_id)
            .fetch_optional(&state.pg)
            .await?;
    tender_id.ok_or_else(|| AppError::NotFound(format!("component {component_id}")))
}

async fn load_component(state: &AppState, component_id: Uuid) -> AppResult<ComponentView> {
    sqlx::query_as(&format!(
        "SELECT {COMPONENT_COLUMNS} FROM components c WHERE c.id = $1"
    ))
    .bind(component_id)
    .fetch_optional(&state.pg)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("component {component_id}")))
}

async fn load_requirement(state: &AppState, requirement_id: Uuid) -> AppResult<RequirementView> {
    sqlx::query_as(&format!(
        "SELECT {REQUIREMENT_COLUMNS} FROM requirements WHERE id = $1"
    ))
    .bind(requirement_id)
    .fetch_optional(&state.pg)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("requirement {requirement_id}")))
}

fn validate_component_input(body: &ComponentInput) -> AppResult<(String, f64, String, String)> {
    let name = body.name.trim().to_string();
    if name.is_empty() || name.chars().count() > 500 {
        return Err(AppError::Validation(
            "position name must be between 1 and 500 characters".into(),
        ));
    }
    let quantity = body.quantity.unwrap_or(1.0);
    if !quantity.is_finite() || quantity <= 0.0 || quantity > 1e9 {
        return Err(AppError::Validation(
            "quantity must be greater than 0 and at most 1e9".into(),
        ));
    }
    let unit = body
        .unit
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("pcs")
        .to_string();
    let category: String = body
        .category
        .as_deref()
        .unwrap_or_default()
        .trim()
        .chars()
        .take(200)
        .collect();
    Ok((name, quantity, unit, category))
}

fn validate_requirement_input(
    body: &RequirementInput,
) -> AppResult<(String, String, serde_json::Value)> {
    let name = body.name.trim().to_string();
    if name.is_empty() || name.chars().count() > 500 {
        return Err(AppError::Validation(
            "requirement name must be between 1 and 500 characters".into(),
        ));
    }
    let data_type = body.data_type.trim().to_uppercase();
    if !DATA_TYPES.contains(&data_type.as_str()) {
        return Err(AppError::Validation(format!(
            "unsupported data_type '{data_type}'; allowed: {}",
            DATA_TYPES.join(", ")
        )));
    }
    if let Some(tolerance) = body.tolerance {
        if !(0.0..=1.0).contains(&tolerance) {
            return Err(AppError::Validation(
                "tolerance must be between 0 and 1".into(),
            ));
        }
    }
    if let (Some(min), Some(max)) = (body.range_min, body.range_max) {
        if min > max {
            return Err(AppError::Validation(
                "range_min must not exceed range_max".into(),
            ));
        }
    }
    let allowed_values = serde_json::json!(body
        .allowed_values
        .iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>());
    Ok((name, data_type, allowed_values))
}

pub async fn list(
    State(state): State<AppState>,
    _user: SessionUser,
    Path(tender_id): Path<Uuid>,
) -> AppResult<Json<Vec<ComponentView>>> {
    let items: Vec<ComponentView> = sqlx::query_as(&format!(
        "SELECT {COMPONENT_COLUMNS} FROM components c
         WHERE c.tender_id = $1 ORDER BY c.position_no, c.id"
    ))
    .bind(tender_id)
    .fetch_all(&state.pg)
    .await?;
    Ok(Json(items))
}

pub async fn create(
    State(state): State<AppState>,
    user: SessionUser,
    headers: HeaderMap,
    Path(tender_id): Path<Uuid>,
    Json(body): Json<ComponentInput>,
) -> AppResult<(StatusCode, Json<ComponentView>)> {
    require_csrf_for(&user, &headers)?;
    ensure_tender_editable(&state, tender_id).await?;
    let (name, quantity, unit, category) = validate_component_input(&body)?;

    let mut tx = state.pg.begin().await?;
    let position_no: i32 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(position_no), 0) + 1 FROM components WHERE tender_id = $1",
    )
    .bind(tender_id)
    .fetch_one(&mut *tx)
    .await?;
    let component_id: Uuid = sqlx::query_scalar(
        "INSERT INTO components
            (tender_id, position_no, name, quantity, unit, category, status, source, search_hints)
         VALUES ($1, $2, $3, $4::float8, $5, $6, 'REVIEWED', 'manual', '{}')
         RETURNING id",
    )
    .bind(tender_id)
    .bind(position_no)
    .bind(&name)
    .bind(quantity)
    .bind(&unit)
    .bind(&category)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;

    sqlx::query(
        "INSERT INTO activity_logs (tender_id, user_id, event_type, message, metadata)
         VALUES ($1, $2, 'component.created', $3, $4)",
    )
    .bind(tender_id)
    .bind(user.id)
    .bind(format!("position '{name}' added manually"))
    .bind(serde_json::json!({ "component_id": component_id.to_string() }))
    .execute(&state.pg)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(load_component(&state, component_id).await?),
    ))
}

pub async fn update(
    State(state): State<AppState>,
    user: SessionUser,
    headers: HeaderMap,
    Path(component_id): Path<Uuid>,
    Json(body): Json<ComponentInput>,
) -> AppResult<Json<ComponentView>> {
    require_csrf_for(&user, &headers)?;
    let tender_id = tender_of_component(&state, component_id).await?;
    ensure_tender_editable(&state, tender_id).await?;
    let (name, quantity, unit, category) = validate_component_input(&body)?;

    sqlx::query(
        "UPDATE components
         SET name = $2, quantity = $3::float8, unit = $4, category = $5
         WHERE id = $1",
    )
    .bind(component_id)
    .bind(&name)
    .bind(quantity)
    .bind(&unit)
    .bind(&category)
    .execute(&state.pg)
    .await?;
    Ok(Json(load_component(&state, component_id).await?))
}

pub async fn remove(
    State(state): State<AppState>,
    user: SessionUser,
    headers: HeaderMap,
    Path(component_id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    require_csrf_for(&user, &headers)?;
    let tender_id = tender_of_component(&state, component_id).await?;
    ensure_tender_editable(&state, tender_id).await?;
    sqlx::query("DELETE FROM components WHERE id = $1")
        .bind(component_id)
        .execute(&state.pg)
        .await?;
    sqlx::query(
        "INSERT INTO activity_logs (tender_id, user_id, event_type, message, metadata)
         VALUES ($1, $2, 'component.deleted', 'position deleted', $3)",
    )
    .bind(tender_id)
    .bind(user.id)
    .bind(serde_json::json!({ "component_id": component_id.to_string() }))
    .execute(&state.pg)
    .await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn list_requirements(
    State(state): State<AppState>,
    _user: SessionUser,
    Path(component_id): Path<Uuid>,
) -> AppResult<Json<Vec<RequirementView>>> {
    let items: Vec<RequirementView> = sqlx::query_as(&format!(
        "SELECT {REQUIREMENT_COLUMNS} FROM requirements
         WHERE component_id = $1 ORDER BY mandatory DESC, name"
    ))
    .bind(component_id)
    .fetch_all(&state.pg)
    .await?;
    Ok(Json(items))
}

pub async fn create_requirement(
    State(state): State<AppState>,
    user: SessionUser,
    headers: HeaderMap,
    Path(component_id): Path<Uuid>,
    Json(body): Json<RequirementInput>,
) -> AppResult<(StatusCode, Json<RequirementView>)> {
    require_csrf_for(&user, &headers)?;
    let tender_id = tender_of_component(&state, component_id).await?;
    ensure_tender_editable(&state, tender_id).await?;
    let (name, data_type, allowed_values) = validate_requirement_input(&body)?;

    let requirement_id: Uuid = sqlx::query_scalar(
        "INSERT INTO requirements
            (component_id, name, value_text, value_num, unit, data_type, mandatory,
             range_min, range_max, tolerance, allowed_values, source)
         VALUES ($1, $2, $3, $4::float8, $5, $6, $7, $8::float8, $9::float8, $10::float8, $11, 'manual')
         RETURNING id",
    )
    .bind(component_id)
    .bind(&name)
    .bind(body.value_text.trim())
    .bind(body.value_num)
    .bind(body.unit.trim())
    .bind(&data_type)
    .bind(body.mandatory)
    .bind(body.range_min)
    .bind(body.range_max)
    .bind(body.tolerance)
    .bind(allowed_values)
    .fetch_one(&state.pg)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(load_requirement(&state, requirement_id).await?),
    ))
}

pub async fn update_requirement(
    State(state): State<AppState>,
    user: SessionUser,
    headers: HeaderMap,
    Path(requirement_id): Path<Uuid>,
    Json(body): Json<RequirementInput>,
) -> AppResult<Json<RequirementView>> {
    require_csrf_for(&user, &headers)?;
    let component_id: Option<Uuid> =
        sqlx::query_scalar("SELECT component_id FROM requirements WHERE id = $1")
            .bind(requirement_id)
            .fetch_optional(&state.pg)
            .await?;
    let component_id =
        component_id.ok_or_else(|| AppError::NotFound(format!("requirement {requirement_id}")))?;
    let tender_id = tender_of_component(&state, component_id).await?;
    ensure_tender_editable(&state, tender_id).await?;
    let (name, data_type, allowed_values) = validate_requirement_input(&body)?;

    sqlx::query(
        "UPDATE requirements
         SET name = $2, value_text = $3, value_num = $4::float8, unit = $5, data_type = $6,
             mandatory = $7, range_min = $8::float8, range_max = $9::float8,
             tolerance = $10::float8, allowed_values = $11
         WHERE id = $1",
    )
    .bind(requirement_id)
    .bind(&name)
    .bind(body.value_text.trim())
    .bind(body.value_num)
    .bind(body.unit.trim())
    .bind(&data_type)
    .bind(body.mandatory)
    .bind(body.range_min)
    .bind(body.range_max)
    .bind(body.tolerance)
    .bind(allowed_values)
    .execute(&state.pg)
    .await?;

    Ok(Json(load_requirement(&state, requirement_id).await?))
}

pub async fn delete_requirement(
    State(state): State<AppState>,
    user: SessionUser,
    headers: HeaderMap,
    Path(requirement_id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    require_csrf_for(&user, &headers)?;
    let component_id: Option<Uuid> =
        sqlx::query_scalar("SELECT component_id FROM requirements WHERE id = $1")
            .bind(requirement_id)
            .fetch_optional(&state.pg)
            .await?;
    let component_id =
        component_id.ok_or_else(|| AppError::NotFound(format!("requirement {requirement_id}")))?;
    let tender_id = tender_of_component(&state, component_id).await?;
    ensure_tender_editable(&state, tender_id).await?;
    sqlx::query("DELETE FROM requirements WHERE id = $1")
        .bind(requirement_id)
        .execute(&state.pg)
        .await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}
