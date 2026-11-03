//! Root health endpoints (`/healthz`, `/readyz`).

use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(readyz))
}

async fn readyz(State(state): State<AppState>) -> impl IntoResponse {
    let db_ok = sqlx::query("SELECT 1").execute(&state.pg).await.is_ok();
    let redis_ok = state.streams.ping().await.is_ok();
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
