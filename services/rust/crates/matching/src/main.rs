//! Matching service entrypoint: gRPC `MatchingService` plus a probe endpoint.
//!
//! The matcher is pure and stateless — no database, no Redis, no network.

use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;

use tender_common::config::Config;
use tender_common::error::AppResult;
use tender_common::telemetry;

use tender_matching::server::MatchingGrpc;
use tender_matching::tender::matching::v1::matching_service_server::MatchingServiceServer;

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        tracing::error!(error = %err, "matching service terminated");
        std::process::exit(1);
    }
}

async fn run() -> AppResult<()> {
    let cfg = Config::from_env()?;
    telemetry::init(&cfg.log_level, cfg.otel_exporter_otlp_endpoint.as_deref());
    tracing::info!("matching service starting");

    let grpc = MatchingGrpc;
    let grpc_addr = "0.0.0.0:50051".parse().expect("static gRPC address");
    tokio::spawn(async move {
        tracing::info!(address = %grpc_addr, "grpc MatchingService listening");
        if let Err(err) = tonic::transport::Server::builder()
            .add_service(MatchingServiceServer::new(grpc))
            .serve(grpc_addr)
            .await
        {
            tracing::error!(error = %err, "grpc server stopped");
        }
    });

    {
        let port = cfg.health_port;
        tokio::spawn(async move {
            let app = Router::new()
                .route("/healthz", get(|| async { "ok" }))
                .route(
                    "/readyz",
                    get(|| async { (axum::http::StatusCode::OK, "ready").into_response() }),
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

    tracing::info!("matching service ready");
    tokio::signal::ctrl_c().await?;
    tracing::info!("shutdown signal received");
    Ok(())
}
