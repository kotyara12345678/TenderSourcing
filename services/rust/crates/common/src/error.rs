//! Unified application error.
//!
//! * HTTP (axum): JSON body `{"error": {"code": "...", "message": "..."}}`
//!   with an appropriate status code.
//! * gRPC (tonic): mapped to canonical status codes.
//!
//! Internal failures (database, redis, storage, io, configuration) are
//! logged with full detail and replaced with a generic message — server
//! internals never reach clients.

use axum::{
    http::{header::RETRY_AFTER, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

/// Message returned for every internal failure, in HTTP and gRPC alike.
const INTERNAL_MESSAGE: &str = "internal server error";

/// Application error shared by every Rust service.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("validation failed: {0}")]
    Validation(String),
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error("not found: {0}")]
    NotFound(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("rate limited, retry after {0} seconds")]
    RateLimited(u64),
    #[error("upstream error: {0}")]
    Upstream(String),
    #[error("timeout: {0}")]
    Timeout(String),
    #[error("storage error: {0}")]
    Storage(String),
    #[error("database error")]
    Database(#[from] sqlx::Error),
    #[error("redis error")]
    Redis(#[from] redis::RedisError),
    #[error("io error")]
    Io(#[from] std::io::Error),
    #[error("configuration error: {0}")]
    Config(#[from] crate::config::ConfigError),
    #[error("{0}")]
    Internal(String),
}

/// Convenience alias used across services.
pub type AppResult<T> = Result<T, AppError>;

impl AppError {
    pub fn internal(message: impl Into<String>) -> Self {
        AppError::Internal(message.into())
    }

    /// Classify the error as
    /// `(status, machine code, client-safe message, retry-after seconds)`.
    /// Internal variants log full detail and expose only a generic message.
    fn http_parts(&self) -> (StatusCode, &'static str, String, Option<u64>) {
        match self {
            AppError::BadRequest(message) => (
                StatusCode::BAD_REQUEST,
                "bad_request",
                message.clone(),
                None,
            ),
            AppError::Validation(message) => {
                (StatusCode::BAD_REQUEST, "validation", message.clone(), None)
            }
            AppError::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                self.to_string(),
                None,
            ),
            AppError::Forbidden => (StatusCode::FORBIDDEN, "forbidden", self.to_string(), None),
            AppError::NotFound(message) => {
                (StatusCode::NOT_FOUND, "not_found", message.clone(), None)
            }
            AppError::Conflict(message) => {
                (StatusCode::CONFLICT, "conflict", message.clone(), None)
            }
            AppError::RateLimited(seconds) => (
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                self.to_string(),
                Some(*seconds),
            ),
            AppError::Upstream(message) => {
                (StatusCode::BAD_GATEWAY, "upstream", message.clone(), None)
            }
            AppError::Timeout(message) => (
                StatusCode::GATEWAY_TIMEOUT,
                "timeout",
                message.clone(),
                None,
            ),
            AppError::Storage(message) => {
                tracing::error!(error = %message, "storage failure");
                internal_part()
            }
            AppError::Database(error) => {
                tracing::error!(error = ?error, "database failure");
                internal_part()
            }
            AppError::Redis(error) => {
                tracing::error!(error = ?error, "redis failure");
                internal_part()
            }
            AppError::Io(error) => {
                tracing::error!(error = ?error, "io failure");
                internal_part()
            }
            AppError::Config(error) => {
                tracing::error!(error = %error, "configuration failure");
                internal_part()
            }
            AppError::Internal(message) => {
                tracing::error!(error = %message, "internal failure");
                internal_part()
            }
        }
    }
}

fn internal_part() -> (StatusCode, &'static str, String, Option<u64>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal",
        INTERNAL_MESSAGE.to_string(),
        None,
    )
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code, message, retry_after) = self.http_parts();
        let mut response = (
            status,
            Json(json!({ "error": { "code": code, "message": message } })),
        )
            .into_response();
        if let Some(seconds) = retry_after {
            if let Ok(value) = axum::http::HeaderValue::try_from(seconds.to_string()) {
                response.headers_mut().insert(RETRY_AFTER, value);
            }
        }
        response
    }
}

impl From<AppError> for tonic::Status {
    fn from(err: AppError) -> Self {
        match &err {
            AppError::BadRequest(message) | AppError::Validation(message) => {
                tonic::Status::invalid_argument(message.clone())
            }
            AppError::Unauthorized => tonic::Status::unauthenticated("unauthorized"),
            AppError::Forbidden => tonic::Status::permission_denied("forbidden"),
            AppError::NotFound(message) => tonic::Status::not_found(message.clone()),
            AppError::Conflict(message) => tonic::Status::already_exists(message.clone()),
            AppError::RateLimited(seconds) => {
                tonic::Status::resource_exhausted(format!("rate limited, retry after {seconds}s"))
            }
            AppError::Upstream(message) => tonic::Status::unavailable(message.clone()),
            AppError::Timeout(message) => tonic::Status::deadline_exceeded(message.clone()),
            AppError::Storage(message) => {
                tracing::error!(error = %message, "storage failure");
                tonic::Status::internal(INTERNAL_MESSAGE)
            }
            AppError::Database(error) => {
                tracing::error!(error = ?error, "database failure");
                tonic::Status::internal(INTERNAL_MESSAGE)
            }
            AppError::Redis(error) => {
                tracing::error!(error = ?error, "redis failure");
                tonic::Status::internal(INTERNAL_MESSAGE)
            }
            AppError::Io(error) => {
                tracing::error!(error = ?error, "io failure");
                tonic::Status::internal(INTERNAL_MESSAGE)
            }
            AppError::Config(error) => {
                tracing::error!(error = %error, "configuration failure");
                tonic::Status::internal(INTERNAL_MESSAGE)
            }
            AppError::Internal(message) => {
                tracing::error!(error = %message, "internal failure");
                tonic::Status::internal(INTERNAL_MESSAGE)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_errors_expose_message() {
        let (status, code, message, retry) =
            AppError::BadRequest("missing name".into()).http_parts();
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(code, "bad_request");
        assert_eq!(message, "missing name");
        assert_eq!(retry, None);
    }

    #[test]
    fn rate_limit_carries_retry_after() {
        let (status, code, _, retry) = AppError::RateLimited(30).http_parts();
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(code, "rate_limited");
        assert_eq!(retry, Some(30));
    }

    #[test]
    fn internal_errors_never_leak_details() {
        let (status, code, message, _) = AppError::Database(sqlx::Error::RowNotFound).http_parts();
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(code, "internal");
        assert_eq!(message, "internal server error");

        let status = tonic::Status::from(AppError::Database(sqlx::Error::RowNotFound));
        assert_eq!(status.code(), tonic::Code::Internal);
        assert_eq!(status.message(), INTERNAL_MESSAGE);
    }

    #[test]
    fn internal_variant_is_generic_too() {
        let (_, code, message, _) = AppError::internal("secret path /srv/x").http_parts();
        assert_eq!(code, "internal");
        assert_eq!(message, "internal server error");
    }

    #[test]
    fn grpc_maps_client_errors() {
        let status = tonic::Status::from(AppError::NotFound("tender 42".into()));
        assert_eq!(status.code(), tonic::Code::NotFound);
        assert_eq!(status.message(), "tender 42");
    }
}
