//! API Gateway entrypoint: migrations, operator bootstrap, HTTP server.

use std::sync::Arc;

use axum::http::{header, HeaderName, HeaderValue, Method};
use tower_http::cors::{AllowOrigin, CorsLayer};

use tender_common::config::Config;
use tender_common::error::{AppError, AppResult};
use tender_common::streams::Streams;
use tender_common::telemetry;

use tender_api::auth;
use tender_api::routes;
use tender_api::state::AppState;

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        tracing::error!(error = %err, "api gateway terminated");
        std::process::exit(1);
    }
}

async fn run() -> AppResult<()> {
    let cfg = Config::from_env()?;
    telemetry::init(&cfg.log_level, cfg.otel_exporter_otlp_endpoint.as_deref());
    tracing::info!(
        env = %cfg.app_env,
        host = %cfg.app_host,
        port = cfg.app_port,
        storage = %cfg.storage_provider,
        "api gateway starting"
    );

    let pg = sqlx::postgres::PgPoolOptions::new()
        .max_connections(cfg.database_max_connections)
        .connect(&cfg.database_url)
        .await?;
    // The API owns schema migrations; workers only ever read/write data.
    sqlx::migrate!("../../../../migrations")
        .run(&pg)
        .await
        .map_err(|err| AppError::internal(format!("database migrations failed: {err}")))?;

    if cfg.storage_provider == "local" {
        tokio::fs::create_dir_all(&cfg.storage_path).await?;
    }
    let streams = Streams::connect(&cfg.redis_url).await?;

    let cors = build_cors(&cfg);
    let body_limit = (cfg.max_file_size as usize).saturating_add(1024 * 1024);
    let cfg = Arc::new(cfg);

    let state = AppState {
        cfg: Arc::clone(&cfg),
        pg,
        streams,
    };
    auth::bootstrap_user(&state).await?;

    let app = routes::router(state)
        .layer(cors)
        .layer(axum::extract::DefaultBodyLimit::max(body_limit));

    let addr = format!("{}:{}", cfg.app_host, cfg.app_port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(address = %addr, "api gateway listening");
    axum::serve(listener, app).await?;
    Ok(())
}

/// Cookie-authenticated API: credentials are allowed only for the explicit
/// origin list from `CORS_ORIGINS` (same-origin deployments need no CORS).
fn build_cors(cfg: &Config) -> CorsLayer {
    let origins: Vec<HeaderValue> = cfg
        .cors_origins
        .iter()
        .filter_map(|origin| origin.parse::<HeaderValue>().ok())
        .collect();

    let layer = CorsLayer::new()
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            header::CONTENT_TYPE,
            header::ACCEPT,
            HeaderName::from_static("x-csrf-token"),
        ])
        .allow_credentials(true);

    if origins.is_empty() {
        layer
    } else {
        layer.allow_origin(AllowOrigin::list(origins))
    }
}
