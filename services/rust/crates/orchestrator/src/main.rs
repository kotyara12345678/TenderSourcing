//! Orchestrator entrypoint.
//!
//! Spawns one consumer loop per stream (control + four event streams), the
//! run watchdog enforcing `MAX_TENDER_DURATION`, and a probe endpoint.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures::future::BoxFuture;
use uuid::Uuid;

use tender_common::config::Config;
use tender_common::error::AppResult;
use tender_common::streams::{self, StreamMessage, Streams};
use tender_common::telemetry;

use tender_orchestrator::control::handle_control;
use tender_orchestrator::handlers::{handle_ai, handle_documents, handle_reports, handle_search};
use tender_orchestrator::state::{OrchestratorState, Outcome};

/// A message that keeps failing is dropped only after this many attempts,
/// always with an ERROR log.
const MAX_ATTEMPTS: u32 = 5;
/// Crash recovery: claim messages abandoned by a dead replica.
const AUTOCLAIM_IDLE_MS: u64 = 600_000;
/// Watchdog tick.
const WATCHDOG_INTERVAL_SECONDS: u64 = 60;

type Handler =
    for<'a> fn(&'a OrchestratorState, &'a StreamMessage) -> BoxFuture<'a, AppResult<Outcome>>;

struct ConsumerSpec {
    stream: &'static str,
    name: &'static str,
    handler: Handler,
}

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        tracing::error!(error = %err, "orchestrator terminated");
        std::process::exit(1);
    }
}

async fn run() -> AppResult<()> {
    let cfg = Config::from_env()?;
    telemetry::init(&cfg.log_level, cfg.otel_exporter_otlp_endpoint.as_deref());
    tracing::info!(
        matching = %cfg.matching_grpc_url,
        max_tender_duration = cfg.max_tender_duration,
        "orchestrator starting"
    );

    let pg = sqlx::postgres::PgPoolOptions::new()
        .max_connections(cfg.database_max_connections)
        .connect(&cfg.database_url)
        .await?;
    let streams = Streams::connect(&cfg.redis_url).await?;

    let cfg = Arc::new(cfg);
    let identity = streams::consumer_identity();

    let specs = [
        ConsumerSpec {
            stream: streams::jobs::CONTROL,
            name: "control",
            handler: handle_control,
        },
        ConsumerSpec {
            stream: streams::events::DOCUMENTS,
            name: "documents",
            handler: handle_documents,
        },
        ConsumerSpec {
            stream: streams::events::AI,
            name: "ai",
            handler: handle_ai,
        },
        ConsumerSpec {
            stream: streams::events::SEARCH,
            name: "search",
            handler: handle_search,
        },
        ConsumerSpec {
            stream: streams::events::REPORTS,
            name: "reports",
            handler: handle_reports,
        },
    ];

    for spec in specs {
        streams
            .ensure_group(spec.stream, streams::groups::ORCHESTRATOR)
            .await?;
        let state = Arc::new(OrchestratorState::new(
            Arc::clone(&cfg),
            pg.clone(),
            streams.clone(),
            format!("{identity}-{}", spec.name),
        ));
        let stream_name = spec.stream;
        let handler = spec.handler;
        tokio::spawn(async move {
            consume(state, stream_name, handler).await;
        });
    }

    // ---------------- watchdog ----------------
    {
        let state = Arc::new(OrchestratorState::new(
            Arc::clone(&cfg),
            pg.clone(),
            streams.clone(),
            format!("{identity}-watchdog"),
        ));
        tokio::spawn(async move { watchdog(state).await });
    }

    // ---------------- health endpoint ----------------
    {
        let pg = pg.clone();
        let streams = streams.clone();
        let port = cfg.health_port;
        tokio::spawn(async move {
            let app = Router::new()
                .route("/healthz", get(|| async { "ok" }))
                .route(
                    "/readyz",
                    get(move || {
                        let pg = pg.clone();
                        let streams = streams.clone();
                        async move {
                            let db_ok = sqlx::query("SELECT 1").execute(&pg).await.is_ok();
                            let redis_ok = streams.ping().await.is_ok();
                            if db_ok && redis_ok {
                                (axum::http::StatusCode::OK, "ready").into_response()
                            } else {
                                (
                                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                                    format!("db={db_ok} redis={redis_ok}"),
                                )
                                    .into_response()
                            }
                        }
                    }),
                );
            let addr = format!("0.0.0.0:{port}");
            match tokio::net::TcpListener::bind(&addr).await {
                Ok(listener) => {
                    tracing::info!(address = %addr, "health endpoint listening");
                    if let Err(err) = axum::serve(listener, app).await {
                        tracing::error!(error = %err, "health server failed");
                    }
                }
                Err(err) => {
                    tracing::error!(error = %err, address = %addr, "health endpoint bind failed")
                }
            }
        });
    }

    tracing::info!("orchestrator ready");
    tokio::signal::ctrl_c().await?;
    tracing::info!("shutdown signal received");
    Ok(())
}

async fn consume(state: Arc<OrchestratorState>, stream_name: &'static str, handler: Handler) {
    let mut attempts: HashMap<String, u32> = HashMap::new();
    tracing::info!(stream = stream_name, consumer = %state.consumer, "consumer loop started");
    loop {
        let mut batch = match state
            .streams
            .autoclaim(
                stream_name,
                streams::groups::ORCHESTRATOR,
                &state.consumer,
                AUTOCLAIM_IDLE_MS,
                8,
            )
            .await
        {
            Ok(messages) => messages,
            Err(err) => {
                tracing::warn!(stream = stream_name, error = %err, "autoclaim failed");
                Vec::new()
            }
        };

        match state
            .streams
            .read(
                stream_name,
                streams::groups::ORCHESTRATOR,
                &state.consumer,
                8,
                5_000,
            )
            .await
        {
            Ok(messages) => batch.extend(messages),
            Err(err) => {
                tracing::error!(stream = stream_name, error = %err, "stream read failed; backing off");
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        }

        for message in &batch {
            match (handler)(&state, message).await {
                Ok(Outcome::Done) => {
                    if let Err(err) = state
                        .streams
                        .ack(
                            stream_name,
                            streams::groups::ORCHESTRATOR,
                            std::slice::from_ref(&message.id),
                        )
                        .await
                    {
                        tracing::warn!(stream = stream_name, error = %err, "ack failed");
                    }
                    attempts.remove(&message.id);
                }
                Err(err) => {
                    tracing::error!(
                        stream = stream_name,
                        message_id = %message.id,
                        error = %err,
                        "handler failed"
                    );
                    let count = {
                        let counter = attempts.entry(message.id.clone()).or_insert(0);
                        *counter += 1;
                        *counter
                    };
                    if count >= MAX_ATTEMPTS {
                        tracing::error!(
                            stream = stream_name,
                            message_id = %message.id,
                            attempts = count,
                            "dropping message after repeated failures"
                        );
                        let _ = state
                            .streams
                            .ack(
                                stream_name,
                                streams::groups::ORCHESTRATOR,
                                std::slice::from_ref(&message.id),
                            )
                            .await;
                        attempts.remove(&message.id);
                    }
                }
            }
        }
    }
}

/// Fail tenders that exceed `MAX_TENDER_DURATION` so a stuck pipeline never
/// holds a tender open forever.
async fn watchdog(state: Arc<OrchestratorState>) {
    let max_duration = i64::try_from(state.cfg.max_tender_duration).unwrap_or(i64::MAX);
    loop {
        tokio::time::sleep(Duration::from_secs(WATCHDOG_INTERVAL_SECONDS)).await;
        let stale: Vec<(Uuid, String, i64)> = match sqlx::query_as(
            "SELECT id, status,
                    EXTRACT(EPOCH FROM (now() - created_at))::bigint AS age_seconds
             FROM tenders
             WHERE deleted_at IS NULL
               AND status NOT IN ('COMPLETED','COMPLETED_WITH_ERRORS','FAILED','CANCELLED')
               AND created_at < now() - ($1 || ' seconds')::interval",
        )
        .bind(max_duration)
        .fetch_all(&state.pg)
        .await
        {
            Ok(rows) => rows,
            Err(err) => {
                tracing::error!(error = %err, "watchdog query failed");
                continue;
            }
        };
        for (tender_id, status, age_seconds) in stale {
            tracing::warn!(
                %tender_id,
                status = %status,
                age_seconds,
                "tender exceeded MAX_TENDER_DURATION; failing it"
            );
            if let Err(err) = state
                .fail(
                    tender_id,
                    &format!("tender exceeded the maximum duration of {max_duration}s"),
                )
                .await
            {
                tracing::error!(error = %err, %tender_id, "watchdog could not fail the tender");
            }
        }
    }
}
