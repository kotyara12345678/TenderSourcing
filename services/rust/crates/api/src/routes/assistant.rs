//! AI intake: the operator writes a request in plain language, the AI worker
//! turns it into positions, and one call materialises them as a tender.
//!
//! The request row is created before the job is queued, so the browser polls a
//! single id from the first millisecond — no "waiting for an id" state.
//! Everything the model produced is only a proposal: the tender is created
//! from what the operator confirmed in [`apply`], never from the model output
//! directly.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use prost::Message as _;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use tender_common::error::{AppError, AppResult};
use tender_common::streams;

use super::require_csrf_for;
use crate::auth::SessionUser;
use crate::state::AppState;
use crate::tender::ai::v1::ParseIntakeJob;

const MAX_TEXT_CHARS: usize = 4_000;
const MIN_TEXT_CHARS: usize = 8;

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

#[derive(Debug, Deserialize)]
pub struct ParseRequest {
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct ParseResponse {
    pub id: Uuid,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct IntakeView {
    pub id: Uuid,
    pub status: String,
    pub progress: f32,
    pub steps: serde_json::Value,
    pub result: Option<serde_json::Value>,
    pub error_message: Option<String>,
    pub tender_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct ApplyRequest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub positions: Vec<ApplyPosition>,
}

#[derive(Debug, Deserialize)]
pub struct ApplyPosition {
    pub name: String,
    pub quantity: Option<f64>,
    #[serde(default)]
    pub unit: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub requirements: Vec<ApplyRequirement>,
}

#[derive(Debug, Deserialize)]
pub struct ApplyRequirement {
    pub name: String,
    #[serde(default)]
    pub value_text: String,
    pub value_num: Option<f64>,
    #[serde(default)]
    pub unit: String,
    pub data_type: Option<String>,
    pub mandatory: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct ApplyResponse {
    pub tender_id: Uuid,
    pub positions: i64,
    pub requirements: i64,
}

/// `POST /assistant/parse` — queue one free-form request.
pub async fn parse(
    State(state): State<AppState>,
    user: SessionUser,
    headers: HeaderMap,
    Json(body): Json<ParseRequest>,
) -> AppResult<(StatusCode, Json<ParseResponse>)> {
    require_csrf_for(&user, &headers)?;

    let text = body.text.trim().to_string();
    let length = text.chars().count();
    if length < MIN_TEXT_CHARS {
        return Err(AppError::Validation(
            "describe what you need in at least a few words".into(),
        ));
    }
    if length > MAX_TEXT_CHARS {
        return Err(AppError::Validation(format!(
            "request is too long: {length} characters, the limit is {MAX_TEXT_CHARS}"
        )));
    }

    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO intake_requests (id, user_id, request_text) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(user.id)
        .bind(&text)
        .execute(&state.pg)
        .await?;

    let job = ParseIntakeJob {
        request_id: id.to_string(),
        text,
    };
    state
        .streams
        .xadd(streams::jobs::AI_INTAKE, &job.encode_to_vec())
        .await?;

    Ok((StatusCode::ACCEPTED, Json(ParseResponse { id })))
}

/// `GET /assistant/{id}` — status, agent steps and the structured result.
pub async fn get(
    State(state): State<AppState>,
    user: SessionUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<IntakeView>> {
    let view: IntakeView = sqlx::query_as(
        "SELECT id, status, progress, steps, result, error_message, tender_id
           FROM intake_requests WHERE id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(user.id)
    .fetch_optional(&state.pg)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("intake request {id}")))?;
    Ok(Json(view))
}

/// `POST /assistant/{id}/apply` — create the tender from confirmed positions.
pub async fn apply(
    State(state): State<AppState>,
    user: SessionUser,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<ApplyRequest>,
) -> AppResult<(StatusCode, Json<ApplyResponse>)> {
    require_csrf_for(&user, &headers)?;

    let status: Option<(String, Option<Uuid>)> = sqlx::query_as(
        "SELECT status, tender_id FROM intake_requests WHERE id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(user.id)
    .fetch_optional(&state.pg)
    .await?;
    let (status, existing_tender) =
        status.ok_or_else(|| AppError::NotFound(format!("intake request {id}")))?;
    if let Some(tender_id) = existing_tender {
        return Err(AppError::Conflict(format!(
            "this request already became tender {tender_id}"
        )));
    }
    if status != "COMPLETED" {
        return Err(AppError::Conflict(format!(
            "the request is in status {status}; it must be parsed before it can become a tender"
        )));
    }

    let name: String = body.name.trim().chars().take(200).collect();
    if name.is_empty() {
        return Err(AppError::Validation("tender name must not be empty".into()));
    }
    if body.positions.is_empty() {
        return Err(AppError::Validation(
            "a tender needs at least one position".into(),
        ));
    }
    if body.positions.len() > state.cfg.max_tender_components as usize {
        return Err(AppError::Validation(format!(
            "too many positions: {}, the limit is {}",
            body.positions.len(),
            state.cfg.max_tender_components
        )));
    }

    // Validate everything before opening a transaction: a bad row must never
    // leave a half-built tender behind.
    struct Position {
        name: String,
        quantity: f64,
        unit: String,
        category: String,
        requirements: Vec<(String, String, Option<f64>, String, String, bool)>,
    }

    let mut positions = Vec::with_capacity(body.positions.len());
    for (index, position) in body.positions.iter().enumerate() {
        let position_name: String = position.name.trim().chars().take(500).collect();
        if position_name.is_empty() {
            return Err(AppError::Validation(format!(
                "position {} has an empty name",
                index + 1
            )));
        }
        let quantity = position.quantity.unwrap_or(1.0);
        if !quantity.is_finite() || quantity <= 0.0 || quantity > 1e9 {
            return Err(AppError::Validation(format!(
                "position {position_name}: quantity must be greater than 0 and at most 1e9"
            )));
        }
        let unit = match position.unit.trim() {
            "" => "pcs".to_string(),
            value => value.chars().take(32).collect(),
        };
        let category: String = position.category.trim().chars().take(200).collect();

        let mut requirements = Vec::with_capacity(position.requirements.len());
        for requirement in &position.requirements {
            let requirement_name: String = requirement.name.trim().chars().take(500).collect();
            if requirement_name.is_empty() {
                continue;
            }
            let data_type = requirement
                .data_type
                .as_deref()
                .unwrap_or("STRING")
                .trim()
                .to_uppercase();
            let data_type = if DATA_TYPES.contains(&data_type.as_str()) {
                data_type
            } else {
                "STRING".to_string()
            };
            requirements.push((
                requirement_name,
                requirement.value_text.trim().chars().take(1000).collect(),
                requirement.value_num,
                requirement.unit.trim().chars().take(32).collect(),
                data_type,
                requirement.mandatory.unwrap_or(false),
            ));
        }

        positions.push(Position {
            name: position_name,
            quantity,
            unit,
            category,
            requirements,
        });
    }

    let description: String = body.description.trim().chars().take(4000).collect();

    let mut tx = state.pg.begin().await?;
    let tender_id: Uuid = sqlx::query_scalar(
        "INSERT INTO tenders (name, description, created_by) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(&name)
    .bind(&description)
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;

    let mut requirement_total: i64 = 0;
    for (index, position) in positions.iter().enumerate() {
        let component_id: Uuid = sqlx::query_scalar(
            "INSERT INTO components
                (tender_id, position_no, name, quantity, unit, category, status, source, search_hints)
             VALUES ($1, $2, $3, $4::float8, $5, $6, 'REVIEWED', 'manual', '{}')
             RETURNING id",
        )
        .bind(tender_id)
        .bind((index + 1) as i32)
        .bind(&position.name)
        .bind(position.quantity)
        .bind(&position.unit)
        .bind(&position.category)
        .fetch_one(&mut *tx)
        .await?;

        for (requirement_name, value_text, value_num, unit, data_type, mandatory) in
            &position.requirements
        {
            sqlx::query(
                "INSERT INTO requirements
                    (component_id, name, value_text, value_num, unit, data_type, mandatory, source, confidence)
                 VALUES ($1, $2, $3, $4::float8, $5, $6, $7, 'manual', 1.000)",
            )
            .bind(component_id)
            .bind(requirement_name)
            .bind(value_text)
            .bind(value_num)
            .bind(unit)
            .bind(data_type)
            .bind(mandatory)
            .execute(&mut *tx)
            .await?;
            requirement_total += 1;
        }
    }

    sqlx::query(
        "UPDATE intake_requests SET status = 'APPLIED', tender_id = $2, progress = 1
          WHERE id = $1",
    )
    .bind(id)
    .bind(tender_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO activity_logs (tender_id, user_id, event_type, message, metadata)
         VALUES ($1, $2, 'tender.created', $3, $4)",
    )
    .bind(tender_id)
    .bind(user.id)
    .bind(format!(
        "tender '{name}' created from a free-form AI request"
    ))
    .bind(serde_json::json!({ "intake_request_id": id.to_string() }))
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok((
        StatusCode::CREATED,
        Json(ApplyResponse {
            tender_id,
            positions: positions.len() as i64,
            requirements: requirement_total,
        }),
    ))
}
