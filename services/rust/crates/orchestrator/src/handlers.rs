//! Event handlers for the `events:*` streams.
//!
//! Every handler is idempotent: the orchestrator may see the same notice
//! twice (at-least-once delivery) and the database writes use upserts,
//! terminal-status guards and `SET NX` guards for outbound jobs.

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use prost::Message;
use serde_json::Value as Json;
use sqlx::FromRow;
use uuid::Uuid;

use tender_common::error::{AppError, AppResult};
use tender_common::streams::{self, StreamMessage};

use crate::fsm::Status;
use crate::state::{OrchestratorState, Outcome};
use crate::tender::ai::v1::{
    ExtractProductsJob, ExtractRequirementsJob, ProductsExtractedNotice,
    RequirementsExtractedNotice,
};
use crate::tender::common::v1::JobEnvelope;
use crate::tender::documents::v1::DocumentExtractedNotice;
use crate::tender::matching::v1::{
    MatchComponentRequest, ProductCandidate, RankOffersRequest, RankedInput, Requirement, SpecValue,
};
use crate::tender::reports::v1::ReportReadyNotice;
use crate::tender::search::v1::SearchPositionResult;

const TERMINAL_TASKS: &str = "'DONE','FAILED','CANCELLED'";
/// Dedup TTL for outbound jobs guarded by the orchestrator.
const QUEUE_GUARD_TTL_SECONDS: u64 = 86_400;
/// Conservative freshness for pages with an unknown retrieval timestamp.
const UNKNOWN_AGE_DAYS: i32 = 365;

type HandlerFuture<'a> = BoxFuture<'a, AppResult<Outcome>>;

// ---------------------------------------------------------------------------
// events:documents
// ---------------------------------------------------------------------------

pub fn handle_documents<'a>(
    state: &'a OrchestratorState,
    message: &'a StreamMessage,
) -> HandlerFuture<'a> {
    Box::pin(async move {
        let notice = match DocumentExtractedNotice::decode(message.payload.as_slice()) {
            Ok(notice) => notice,
            Err(err) => {
                tracing::error!(error = %err, "undecodable DocumentExtractedNotice — dropping");
                return Ok(Outcome::Done);
            }
        };
        let Some(envelope) = notice.envelope.as_ref() else {
            tracing::error!("DocumentExtractedNotice without envelope — dropping");
            return Ok(Outcome::Done);
        };
        let Ok(tender_id) = Uuid::parse_str(&envelope.tender_id) else {
            tracing::error!(tender_id = %envelope.tender_id, "invalid tender id — dropping");
            return Ok(Outcome::Done);
        };
        if state.tender_status(tender_id).await?.is_terminal() {
            return Ok(Outcome::Done);
        }
        if !notice.success {
            tracing::warn!(
                document_id = %notice.document_id,
                error = ?notice.error_message,
                "document extraction failed"
            );
        }

        let pending: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM documents
             WHERE tender_id = $1 AND deleted_at IS NULL
               AND status IN ('UPLOADED','QUEUED','PROCESSING')",
        )
        .bind(tender_id)
        .fetch_one(&state.pg)
        .await?;
        if pending > 0 {
            return Ok(Outcome::Done);
        }

        let documents: Vec<(Uuid, String)> = sqlx::query_as(
            "SELECT id, original_filename FROM documents
             WHERE tender_id = $1 AND deleted_at IS NULL AND status = 'EXTRACTED'
             ORDER BY uploaded_at, id",
        )
        .bind(tender_id)
        .fetch_all(&state.pg)
        .await?;

        if documents.is_empty() {
            state
                .fail(
                    tender_id,
                    "no document could be parsed; requirements cannot be extracted",
                )
                .await?;
            return Ok(Outcome::Done);
        }

        state
            .advance(tender_id, Status::RequirementExtraction)
            .await?;
        state
            .activity(
                tender_id,
                "documents.processed",
                &format!("{} document(s) parsed", documents.len()),
                Json::Object(Default::default()),
            )
            .await?;

        for (document_id, _name) in &documents {
            let key = format!("ai:requirements:queued:{document_id}");
            if !state.streams.set_nx(&key, QUEUE_GUARD_TTL_SECONDS).await? {
                continue;
            }
            let job = ExtractRequirementsJob {
                envelope: Some(job_envelope(tender_id, None, Uuid::new_v4())),
                document_id: document_id.to_string(),
                escalate: false,
            };
            state
                .streams
                .xadd_kind(
                    streams::jobs::AI_REQUIREMENTS,
                    "requirements",
                    &job.encode_to_vec(),
                )
                .await?;
        }
        state
            .progress(
                tender_id,
                None,
                None,
                Status::RequirementExtraction,
                0.0,
                "info",
                format!(
                    "extracting requirements from {} document(s)",
                    documents.len()
                ),
            )
            .await;
        Ok(Outcome::Done)
    })
}

// ---------------------------------------------------------------------------
// events:ai (kinds: requirements | products)
// ---------------------------------------------------------------------------

pub fn handle_ai<'a>(
    state: &'a OrchestratorState,
    message: &'a StreamMessage,
) -> HandlerFuture<'a> {
    Box::pin(async move {
        match message.kind.as_str() {
            "requirements" => handle_requirements(state, message).await,
            "products" => handle_products(state, message).await,
            other => {
                tracing::warn!(kind = other, "unknown events:ai kind — dropping");
                Ok(Outcome::Done)
            }
        }
    })
}

async fn handle_requirements(
    state: &OrchestratorState,
    message: &StreamMessage,
) -> AppResult<Outcome> {
    let notice = match RequirementsExtractedNotice::decode(message.payload.as_slice()) {
        Ok(notice) => notice,
        Err(err) => {
            tracing::error!(error = %err, "undecodable RequirementsExtractedNotice — dropping");
            return Ok(Outcome::Done);
        }
    };
    let Some(envelope) = notice.envelope.as_ref() else {
        return Ok(Outcome::Done);
    };
    let Ok(tender_id) = Uuid::parse_str(&envelope.tender_id) else {
        return Ok(Outcome::Done);
    };
    if state.tender_status(tender_id).await?.is_terminal() {
        return Ok(Outcome::Done);
    }
    if !notice.success {
        tracing::warn!(
            document_id = %notice.document_id,
            error = %notice.error_message,
            "requirement extraction failed for one document"
        );
    }

    let still_extracting: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM documents
         WHERE tender_id = $1 AND deleted_at IS NULL AND status = 'EXTRACTED'
           AND ai_status IN ('PENDING','RUNNING')",
    )
    .bind(tender_id)
    .fetch_one(&state.pg)
    .await?;
    if still_extracting > 0 {
        return Ok(Outcome::Done);
    }

    let components: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM components WHERE tender_id = $1")
            .bind(tender_id)
            .fetch_one(&state.pg)
            .await?;

    state.advance(tender_id, Status::RequirementsReview).await?;
    state
        .activity(
            tender_id,
            "requirements.extracted",
            &format!("{components} position(s) extracted; awaiting review"),
            serde_json::json!({ "components": components }),
        )
        .await?;
    state
        .progress(
            tender_id,
            None,
            None,
            Status::RequirementsReview,
            1.0,
            "success",
            format!("{components} position(s) ready for review"),
        )
        .await;
    Ok(Outcome::Done)
}

async fn handle_products(state: &OrchestratorState, message: &StreamMessage) -> AppResult<Outcome> {
    let notice = match ProductsExtractedNotice::decode(message.payload.as_slice()) {
        Ok(notice) => notice,
        Err(err) => {
            tracing::error!(error = %err, "undecodable ProductsExtractedNotice — dropping");
            return Ok(Outcome::Done);
        }
    };
    let Some(envelope) = notice.envelope.as_ref() else {
        return Ok(Outcome::Done);
    };
    let (Ok(task_id), Ok(run_id), Ok(tender_id)) = (
        Uuid::parse_str(&envelope.task_id),
        Uuid::parse_str(&envelope.run_id),
        Uuid::parse_str(&envelope.tender_id),
    ) else {
        tracing::error!("products notice has invalid ids — dropping");
        return Ok(Outcome::Done);
    };
    if state.tender_status(tender_id).await?.is_terminal() {
        return Ok(Outcome::Done);
    }
    let Ok(component_id) = Uuid::parse_str(&notice.component_id) else {
        tracing::error!("products notice has invalid component id — dropping");
        return Ok(Outcome::Done);
    };

    if !notice.success {
        let message = if notice.error_message.is_empty() {
            "product extraction failed".to_string()
        } else {
            notice.error_message.clone()
        };
        fail_task(state, task_id, component_id, run_id, &message).await?;
    } else {
        sqlx::query(
            "UPDATE search_tasks SET status = 'MATCHING', updated_at = now()
             WHERE id = $1 AND status NOT IN ('DONE','FAILED','CANCELLED')",
        )
        .bind(task_id)
        .execute(&state.pg)
        .await?;

        match match_and_rank(state, tender_id, run_id, component_id, task_id).await {
            Ok((matched, ranked)) => {
                sqlx::query(
                    "UPDATE search_tasks SET status = 'DONE', finished_at = now(), updated_at = now()
                     WHERE id = $1 AND status NOT IN ('DONE','FAILED','CANCELLED')",
                )
                .bind(task_id)
                .execute(&state.pg)
                .await?;
                sqlx::query(
                    "UPDATE components SET status = 'MATCHED', error_message = NULL WHERE id = $1",
                )
                .bind(component_id)
                .execute(&state.pg)
                .await?;
                sqlx::query(
                    "UPDATE search_runs SET components_done = components_done + 1, updated_at = now()
                     WHERE id = $1",
                )
                .bind(run_id)
                .execute(&state.pg)
                .await?;
                state
                    .progress(
                        tender_id,
                        Some(run_id),
                        Some(task_id),
                        Status::Ranking,
                        1.0,
                        "success",
                        format!("{matched} match(es), {ranked} ranked offer(s)"),
                    )
                    .await;
            }
            Err(err) => {
                tracing::error!(
                    %task_id,
                    error = %err,
                    "matching/ranking failed for a position"
                );
                fail_task(state, task_id, component_id, run_id, &err.to_string()).await?;
            }
        }
    }

    state.refresh_milestone(tender_id, run_id).await?;
    Ok(Outcome::Done)
}

async fn fail_task(
    state: &OrchestratorState,
    task_id: Uuid,
    component_id: Uuid,
    run_id: Uuid,
    message: &str,
) -> AppResult<()> {
    sqlx::query(
        "UPDATE search_tasks SET status = 'FAILED', last_error = $2, finished_at = now(), updated_at = now()
         WHERE id = $1 AND status NOT IN ('DONE','FAILED','CANCELLED')",
    )
    .bind(task_id)
    .bind(message)
    .execute(&state.pg)
    .await?;
    sqlx::query(
        "UPDATE components SET status = 'FAILED', error_message = $2
         WHERE id = $1 AND status NOT IN ('MATCHED','SKIPPED')",
    )
    .bind(component_id)
    .bind(message)
    .execute(&state.pg)
    .await?;
    sqlx::query(
        "UPDATE search_runs SET components_failed = components_failed + 1, updated_at = now()
         WHERE id = $1",
    )
    .bind(run_id)
    .execute(&state.pg)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// events:search
// ---------------------------------------------------------------------------

pub fn handle_search<'a>(
    state: &'a OrchestratorState,
    message: &'a StreamMessage,
) -> HandlerFuture<'a> {
    Box::pin(async move {
        let notice = match SearchPositionResult::decode(message.payload.as_slice()) {
            Ok(notice) => notice,
            Err(err) => {
                tracing::error!(error = %err, "undecodable SearchPositionResult — dropping");
                return Ok(Outcome::Done);
            }
        };
        let Some(envelope) = notice.envelope.as_ref() else {
            return Ok(Outcome::Done);
        };
        let (Ok(task_id), Ok(run_id), Ok(tender_id), Ok(component_id)) = (
            Uuid::parse_str(&envelope.task_id),
            Uuid::parse_str(&envelope.run_id),
            Uuid::parse_str(&envelope.tender_id),
            Uuid::parse_str(&notice.component_id),
        ) else {
            tracing::error!("search notice has invalid ids — dropping");
            return Ok(Outcome::Done);
        };
        if state.tender_status(tender_id).await?.is_terminal() {
            return Ok(Outcome::Done);
        }

        if !notice.success {
            fail_task(state, task_id, component_id, run_id, &notice.error_message).await?;
            state.refresh_milestone(tender_id, run_id).await?;
            return Ok(Outcome::Done);
        }

        sqlx::query(
            "UPDATE search_tasks SET status = 'EXTRACTING', updated_at = now()
             WHERE id = $1 AND status NOT IN ('DONE','FAILED','CANCELLED')",
        )
        .bind(task_id)
        .execute(&state.pg)
        .await?;
        sqlx::query(
            "UPDATE components SET status = 'SEARCHED' WHERE id = $1 AND status NOT IN ('MATCHED','SKIPPED')",
        )
        .bind(component_id)
        .execute(&state.pg)
        .await?;

        let key = format!("ai:products:queued:{task_id}");
        if state.streams.set_nx(&key, QUEUE_GUARD_TTL_SECONDS).await? {
            let job = ExtractProductsJob {
                envelope: Some(job_envelope(tender_id, Some(run_id), task_id)),
                component_id: component_id.to_string(),
                escalate: false,
            };
            state
                .streams
                .xadd_kind(streams::jobs::AI_PRODUCTS, "products", &job.encode_to_vec())
                .await?;
            state
                .progress(
                    tender_id,
                    Some(run_id),
                    Some(task_id),
                    Status::ProductExtraction,
                    0.0,
                    "info",
                    "extracting product specifications from fetched pages",
                )
                .await;
        }

        state.refresh_milestone(tender_id, run_id).await?;
        Ok(Outcome::Done)
    })
}

// ---------------------------------------------------------------------------
// events:reports
// ---------------------------------------------------------------------------

pub fn handle_reports<'a>(
    state: &'a OrchestratorState,
    message: &'a StreamMessage,
) -> HandlerFuture<'a> {
    Box::pin(async move {
        let notice = match ReportReadyNotice::decode(message.payload.as_slice()) {
            Ok(notice) => notice,
            Err(err) => {
                tracing::error!(error = %err, "undecodable ReportReadyNotice — dropping");
                return Ok(Outcome::Done);
            }
        };
        let (Ok(tender_id), Ok(run_id)) = (
            Uuid::parse_str(&notice.tender_id),
            Uuid::parse_str(&notice.run_id),
        ) else {
            tracing::error!("report notice has invalid ids — dropping");
            return Ok(Outcome::Done);
        };
        if state.tender_status(tender_id).await?.is_terminal() {
            return Ok(Outcome::Done);
        }

        if !notice.success {
            let reason = if notice.error_message.is_empty() {
                "report generation failed".to_string()
            } else {
                format!("report generation failed: {}", notice.error_message)
            };
            state.fail(tender_id, &reason).await?;
            return Ok(Outcome::Done);
        }

        state.advance(tender_id, Status::ReportGeneration).await?;
        let failed_tasks: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM search_tasks WHERE run_id = $1 AND status = 'FAILED'",
        )
        .bind(run_id)
        .fetch_one(&state.pg)
        .await?;
        let target = if failed_tasks > 0 {
            Status::CompletedWithErrors
        } else {
            Status::Completed
        };
        let reached = state.apply(tender_id, target).await?;

        sqlx::query(
            "UPDATE search_runs SET status = $2, finished_at = now(), updated_at = now()
             WHERE id = $1 AND status IN ('QUEUED','RUNNING')",
        )
        .bind(run_id)
        .bind(reached.as_str())
        .execute(&state.pg)
        .await?;

        state
            .activity(
                tender_id,
                "tender.completed",
                if failed_tasks > 0 {
                    "tender completed with failing positions"
                } else {
                    "tender completed"
                },
                serde_json::json!({ "run_id": run_id.to_string(), "failed_positions": failed_tasks }),
            )
            .await?;
        state
            .progress(
                tender_id,
                Some(run_id),
                None,
                reached,
                1.0,
                if failed_tasks > 0 {
                    "warning"
                } else {
                    "success"
                },
                format!("{} file(s) generated", notice.files_created),
            )
            .await;
        Ok(Outcome::Done)
    })
}

// ---------------------------------------------------------------------------
// Matching + ranking step (orchestrator -> matching gRPC -> PostgreSQL)
// ---------------------------------------------------------------------------

#[derive(Debug, FromRow)]
struct RequirementRow {
    id: Uuid,
    name: String,
    value_text: String,
    value_num: f64,
    unit: String,
    data_type: String,
    mandatory: bool,
    range_min: f64,
    range_max: f64,
    tolerance: f64,
    allowed_values: Json,
    source: String,
    confidence: f64,
}

#[derive(Debug, FromRow)]
struct ProductRow {
    id: Uuid,
    price: Option<f64>,
    currency: Option<String>,
    lead_time_days: Option<i32>,
    supplier_reliability: Option<f64>,
    retrieved_at: Option<DateTime<Utc>>,
}

#[derive(Debug, FromRow)]
struct SpecRow {
    product_id: Uuid,
    id: Uuid,
    name: String,
    value_text: String,
    value_num: f64,
    unit: String,
    data_type: String,
    source_id: Option<Uuid>,
    source_snippet: String,
    confidence: f64,
}

/// Run the deterministic matcher and the explainable ranker over gRPC and
/// persist `matches` / `match_details` / ranking columns.
/// Returns `(matched_products, ranked_offers)`.
async fn match_and_rank(
    state: &OrchestratorState,
    tender_id: Uuid,
    run_id: Uuid,
    component_id: Uuid,
    task_id: Uuid,
) -> AppResult<(usize, usize)> {
    let requirements: Vec<RequirementRow> = sqlx::query_as(
        "SELECT id, name, value_text, COALESCE(value_num, 0)::float8 AS value_num, unit,
                data_type, mandatory, COALESCE(range_min, 0)::float8 AS range_min,
                COALESCE(range_max, 0)::float8 AS range_max,
                COALESCE(tolerance, 0)::float8 AS tolerance, allowed_values, source,
                confidence::float8 AS confidence
         FROM requirements WHERE component_id = $1 ORDER BY name",
    )
    .bind(component_id)
    .fetch_all(&state.pg)
    .await?;

    let proto_requirements: Vec<Requirement> = requirements
        .iter()
        .map(|row| Requirement {
            id: row.id.to_string(),
            name: row.name.clone(),
            value_text: row.value_text.clone(),
            value_num: row.value_num,
            unit: row.unit.clone(),
            data_type: row.data_type.clone(),
            mandatory: row.mandatory,
            range_min: row.range_min,
            range_max: row.range_max,
            tolerance: row.tolerance,
            allowed_values: row
                .allowed_values
                .as_array()
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Json::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            source: row.source.clone(),
            confidence: row.confidence,
        })
        .collect();

    let products: Vec<ProductRow> = sqlx::query_as(
        "SELECT p.id, p.price::float8 AS price, p.currency::text AS currency,
                p.lead_time_days,
                sup.reliability_score::float8 AS supplier_reliability,
                s.retrieved_at
         FROM products p
         LEFT JOIN suppliers sup ON sup.id = p.supplier_id
         LEFT JOIN sources s ON s.id = p.source_id
         WHERE p.source_id IN (
             SELECT source_id FROM search_results
             WHERE task_id = $1 AND source_id IS NOT NULL
         )
         ORDER BY p.created_at, p.id",
    )
    .bind(task_id)
    .fetch_all(&state.pg)
    .await?;

    if products.is_empty() {
        state
            .progress(
                tender_id,
                Some(run_id),
                Some(task_id),
                Status::Matching,
                0.5,
                "warning",
                "no product candidates were extracted from the fetched pages",
            )
            .await;
        return Ok((0, 0));
    }

    let product_ids: Vec<Uuid> = products.iter().map(|product| product.id).collect();
    let specs: Vec<SpecRow> = sqlx::query_as(
        "SELECT product_id, id, name, value_text, COALESCE(value_num, 0)::float8 AS value_num,
                unit, data_type, source_id, source_snippet, confidence::float8 AS confidence
         FROM product_specs WHERE product_id = ANY($1) ORDER BY product_id, name",
    )
    .bind(&product_ids)
    .fetch_all(&state.pg)
    .await?;

    let candidates: Vec<ProductCandidate> = products
        .iter()
        .map(|product| ProductCandidate {
            product_id: product.id.to_string(),
            specs: specs
                .iter()
                .filter(|spec| spec.product_id == product.id)
                .map(|spec| SpecValue {
                    id: spec.id.to_string(),
                    name: spec.name.clone(),
                    value_text: spec.value_text.clone(),
                    value_num: spec.value_num,
                    unit: spec.unit.clone(),
                    data_type: spec.data_type.clone(),
                    source_id: spec.source_id.map(|id| id.to_string()).unwrap_or_default(),
                    source_snippet: spec.source_snippet.clone(),
                    confidence: spec.confidence,
                })
                .collect(),
        })
        .collect();

    let mut client = state.matching_client().await?;
    let response = client
        .match_component(MatchComponentRequest {
            component_id: component_id.to_string(),
            requirements: proto_requirements,
            products: candidates,
        })
        .await
        .map_err(|status| AppError::Upstream(format!("matching grpc: {}", status.message())))?
        .into_inner();

    let now = Utc::now();
    let mut eligible_offers: Vec<RankedInput> = Vec::new();
    let mut matched_products = 0usize;

    for matched in &response.matches {
        let Ok(product_id) = Uuid::parse_str(&matched.product_id) else {
            continue;
        };
        let Some(product) = products.iter().find(|candidate| candidate.id == product_id) else {
            continue;
        };
        let verdict = verdict_text(matched.verdict);
        let match_id: Uuid = sqlx::query_scalar(
            "INSERT INTO matches
                (component_id, product_id, run_id, verdict, eligible, technical_score)
             VALUES ($1,$2,$3,$4,$5,$6::float8)
             ON CONFLICT (run_id, component_id, product_id)
             DO UPDATE SET verdict = EXCLUDED.verdict, eligible = EXCLUDED.eligible,
                           technical_score = EXCLUDED.technical_score, updated_at = now()
             RETURNING id",
        )
        .bind(component_id)
        .bind(product_id)
        .bind(run_id)
        .bind(verdict)
        .bind(matched.eligible)
        .bind(matched.technical_score)
        .fetch_one(&state.pg)
        .await?;

        sqlx::query("DELETE FROM match_details WHERE match_id = $1")
            .bind(match_id)
            .execute(&state.pg)
            .await?;
        for check in &matched.checks {
            sqlx::query(
                "INSERT INTO match_details
                    (match_id, requirement_id, requirement_name, status, expected, actual, unit, explanation)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
            )
            .bind(match_id)
            .bind(Uuid::parse_str(&check.requirement_id).ok())
            .bind(&check.requirement_name)
            .bind(check_status(&check.status))
            .bind(&check.expected)
            .bind(&check.actual)
            .bind(&check.unit)
            .bind(&check.explanation)
            .execute(&state.pg)
            .await?;
        }
        matched_products += 1;

        if matched.eligible {
            let age_days = product.retrieved_at.map_or(UNKNOWN_AGE_DAYS, |retrieved| {
                now.signed_duration_since(retrieved).num_days().max(0) as i32
            });
            eligible_offers.push(RankedInput {
                product_id: matched.product_id.clone(),
                r#match: Some(matched.clone()),
                has_price: product.price.is_some_and(|price| price > 0.0),
                price: product.price.unwrap_or(0.0),
                currency: product.currency.clone().unwrap_or_default(),
                has_lead_time: product.lead_time_days.is_some(),
                lead_time_days: product.lead_time_days.unwrap_or(0),
                has_supplier_reliability: product.supplier_reliability.is_some(),
                supplier_reliability: product.supplier_reliability.unwrap_or(0.0),
                data_age_days: age_days,
            });
        }
    }

    if eligible_offers.is_empty() {
        return Ok((matched_products, 0));
    }

    let ranked = client
        .rank_offers(RankOffersRequest {
            component_id: component_id.to_string(),
            offers: eligible_offers,
            weights: None,
        })
        .await
        .map_err(|status| AppError::Upstream(format!("ranking grpc: {}", status.message())))?
        .into_inner();

    for (index, offer) in ranked.offers.iter().enumerate() {
        let Ok(product_id) = Uuid::parse_str(&offer.product_id) else {
            continue;
        };
        let breakdown: Vec<Json> = offer
            .breakdown
            .iter()
            .map(|component| {
                serde_json::json!({
                    "factor": component.factor,
                    "weight": component.weight,
                    "normalized": component.normalized,
                    "weighted": component.weighted,
                    "note": component.note,
                })
            })
            .collect();
        sqlx::query(
            "UPDATE matches
             SET rank = $4, total_score = $5::float8, score_breakdown = $6, explanation = $7,
                 updated_at = now()
             WHERE run_id = $1 AND component_id = $2 AND product_id = $3",
        )
        .bind(run_id)
        .bind(component_id)
        .bind(product_id)
        .bind((index + 1) as i32)
        .bind(offer.total_score)
        .bind(Json::Array(breakdown))
        .bind(&offer.summary)
        .execute(&state.pg)
        .await?;
    }

    Ok((matched_products, ranked.offers.len()))
}

fn verdict_text(verdict: i32) -> &'static str {
    match verdict {
        1 => "FULL",
        2 => "TOLERANCE",
        3 => "PARTIAL",
        4 => "MISMATCH",
        _ => "INSUFFICIENT",
    }
}

/// `match_details.status` CHECK: match | tolerance | partial | mismatch | missing.
fn check_status(status: &str) -> &'static str {
    match status {
        "match" => "match",
        "tolerance" => "tolerance",
        "mismatch" => "mismatch",
        "missing" => "missing",
        _ => "partial",
    }
}

fn job_envelope(tender_id: Uuid, run_id: Option<Uuid>, task_id: Uuid) -> JobEnvelope {
    JobEnvelope {
        task_id: task_id.to_string(),
        tender_id: tender_id.to_string(),
        run_id: run_id.map(|id| id.to_string()).unwrap_or_default(),
        created_at_unix_ms: Utc::now().timestamp_millis(),
        attempt: 1,
        metadata: Default::default(),
    }
}

/// Keep the constant referenced from SQL fragments in one place.
pub fn terminal_tasks_sql() -> &'static str {
    TERMINAL_TASKS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdict_mapping_covers_all_enum_values() {
        assert_eq!(verdict_text(1), "FULL");
        assert_eq!(verdict_text(2), "TOLERANCE");
        assert_eq!(verdict_text(3), "PARTIAL");
        assert_eq!(verdict_text(4), "MISMATCH");
        assert_eq!(verdict_text(5), "INSUFFICIENT");
        // Unspecified (0) degrades to the conservative verdict.
        assert_eq!(verdict_text(0), "INSUFFICIENT");
        assert_eq!(verdict_text(99), "INSUFFICIENT");
    }

    #[test]
    fn check_status_is_constrained_to_db_values() {
        for status in ["match", "tolerance", "mismatch", "missing", "partial"] {
            assert_eq!(check_status(status), status);
        }
        assert_eq!(check_status("weird"), "partial");
    }

    #[test]
    fn job_envelope_carries_ids() {
        let tender = Uuid::new_v4();
        let run = Uuid::new_v4();
        let task = Uuid::new_v4();
        let envelope = job_envelope(tender, Some(run), task);
        assert_eq!(envelope.tender_id, tender.to_string());
        assert_eq!(envelope.run_id, run.to_string());
        assert_eq!(envelope.task_id, task.to_string());
        assert!(envelope.created_at_unix_ms > 0);

        let without_run = job_envelope(tender, None, task);
        assert!(without_run.run_id.is_empty());
    }

    #[test]
    fn terminal_tasks_sql_matches_db_statuses() {
        assert!(terminal_tasks_sql().contains("DONE"));
        assert!(terminal_tasks_sql().contains("FAILED"));
        assert!(terminal_tasks_sql().contains("CANCELLED"));
    }
}
