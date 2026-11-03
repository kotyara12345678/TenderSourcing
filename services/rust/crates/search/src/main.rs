//! Search service entrypoint: gRPC `SearchService`, the `jobs:search`
//! consumer workers, and a `/healthz` `/readyz` probe endpoint.

use std::sync::Arc;

use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;

use tender_common::config::Config;
use tender_common::error::AppResult;
use tender_common::streams::{self, Streams};
use tender_common::telemetry;

use tender_search::client::KeenableClient;
use tender_search::fetch::FetchBackend;
use tender_search::jobs::SearchWorker;
use tender_search::server::SearchGrpc;
use tender_search::tender::search::v1::search_service_server::SearchServiceServer;

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        tracing::error!(error = %err, "search service terminated");
        std::process::exit(1);
    }
}

async fn run() -> AppResult<()> {
    let cfg = Config::from_env()?;
    telemetry::init(&cfg.log_level, cfg.otel_exporter_otlp_endpoint.as_deref());
    // Fail fast: the search engine cannot work without a provider key.
    cfg.require_keenable_api_key()?;
    tracing::info!(
        mode = %cfg.keenable_search_mode,
        rps = cfg.keenable_max_rps,
        fetch_backend = %cfg.fetch_backend,
        "search service starting"
    );

    let pg = sqlx::postgres::PgPoolOptions::new()
        .max_connections(cfg.database_max_connections)
        .connect(&cfg.database_url)
        .await?;
    let streams = Streams::connect(&cfg.redis_url).await?;
    streams
        .ensure_group(streams::jobs::SEARCH, streams::groups::SEARCH_WORKERS)
        .await?;

    let cfg = Arc::new(cfg);
    let client = Arc::new(KeenableClient::new(&cfg)?);
    let backend = Arc::new(FetchBackend::from_config(&cfg, Arc::clone(&client))?);

    // ---------------- gRPC ----------------
    let grpc = SearchGrpc {
        client: Arc::clone(&client),
        backend: Arc::clone(&backend),
        streams: streams.clone(),
    };
    let grpc_addr = "0.0.0.0:50052".parse().expect("static gRPC address");
    tokio::spawn(async move {
        tracing::info!(address = %grpc_addr, "grpc SearchService listening");
        if let Err(err) = tonic::transport::Server::builder()
            .add_service(SearchServiceServer::new(grpc))
            .serve(grpc_addr)
            .await
        {
            tracing::error!(error = %err, "grpc server stopped");
        }
    });

    // ---------------- health endpoint ----------------
    {
        let pg = pg.clone();
        let streams = streams.clone();
        let client = Arc::clone(&client);
        let port = cfg.health_port;
        tokio::spawn(async move {
            let app = Router::new()
                .route("/healthz", get(|| async { "ok" }))
                .route(
                    "/readyz",
                    get(move || {
                        let pg = pg.clone();
                        let streams = streams.clone();
                        let client = Arc::clone(&client);
                        async move {
                            let db_ok = sqlx::query("SELECT 1").execute(&pg).await.is_ok();
                            let redis_ok = streams.ping().await.is_ok();
                            let provider_ok = client.probe_public().await.is_ok();
                            if db_ok && redis_ok && provider_ok {
                                (axum::http::StatusCode::OK, "ready").into_response()
                            } else {
                                (
                                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                                    format!("db={db_ok} redis={redis_ok} provider={provider_ok}"),
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

    // ---------------- stream workers ----------------
    let workers = cfg.search_concurrency.max(1);
    let identity = streams::consumer_identity();
    for index in 0..workers {
        let worker = Arc::new(SearchWorker::new(
            Arc::clone(&cfg),
            pg.clone(),
            streams.clone(),
            Arc::clone(&client),
            Arc::clone(&backend),
            format!("{identity}-{index}"),
        ));
        tokio::spawn(async move {
            if let Err(err) = worker.run().await {
                tracing::error!(error = %err, "search worker loop exited");
            }
        });
    }

    tracing::info!(workers, "search service ready");
    tokio::signal::ctrl_c().await?;
    tracing::info!("shutdown signal received");
    Ok(())
}
