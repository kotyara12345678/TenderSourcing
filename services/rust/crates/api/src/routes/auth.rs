//! Auth endpoints.

use axum::extract::State;
use axum::http::{header, HeaderMap};
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use tender_common::error::{AppError, AppResult};

use super::require_csrf_for;
use crate::auth::{self, SessionUser};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct UserView {
    pub id: Uuid,
    pub username: String,
    pub display_name: String,
    pub csrf_token: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub user: UserView,
}

pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<LoginRequest>,
) -> AppResult<impl IntoResponse> {
    let username = body.username.trim();
    if username.is_empty() || body.password.is_empty() {
        return Err(AppError::Validation(
            "username and password are required".into(),
        ));
    }

    let row: Option<(Uuid, String, String, String, bool)> = sqlx::query_as(
        "SELECT id, username, display_name, password_hash, is_active
         FROM users WHERE username = $1 AND deleted_at IS NULL",
    )
    .bind(username)
    .fetch_optional(&state.pg)
    .await?;

    // Uniform failure for wrong username and wrong password.
    let Some((user_id, stored_username, display_name, password_hash, is_active)) = row else {
        return Err(AppError::Unauthorized);
    };
    if !is_active || !auth::verify_password(&body.password, &password_hash) {
        return Err(AppError::Unauthorized);
    }

    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let issued = auth::create_session(&state, user_id, &user_agent, None).await?;

    sqlx::query(
        "INSERT INTO activity_logs (tender_id, user_id, event_type, message, metadata)
         VALUES (NULL, $1, 'auth.login', 'operator signed in', '{}')",
    )
    .bind(user_id)
    .execute(&state.pg)
    .await?;

    let cookie = auth::session_cookie(&state, &issued.token);
    Ok((
        [(header::SET_COOKIE, cookie)],
        Json(LoginResponse {
            user: UserView {
                id: user_id,
                username: stored_username,
                display_name,
                csrf_token: issued.csrf_token,
                expires_at: issued.expires_at,
            },
        }),
    ))
}

pub async fn logout(
    State(state): State<AppState>,
    user: SessionUser,
    headers: HeaderMap,
) -> AppResult<impl IntoResponse> {
    require_csrf_for(&user, &headers)?;
    sqlx::query("UPDATE sessions SET revoked_at = now() WHERE id = $1")
        .bind(user.session_id)
        .execute(&state.pg)
        .await?;
    sqlx::query(
        "INSERT INTO activity_logs (tender_id, user_id, event_type, message, metadata)
         VALUES (NULL, $1, 'auth.logout', 'operator signed out', '{}')",
    )
    .bind(user.id)
    .execute(&state.pg)
    .await?;
    let cookie = auth::clear_cookie(&state);
    Ok((
        [(header::SET_COOKIE, cookie)],
        Json(serde_json::json!({ "ok": true })),
    ))
}

pub async fn me(user: SessionUser) -> AppResult<Json<UserView>> {
    Ok(Json(UserView {
        id: user.id,
        username: user.username,
        display_name: user.display_name,
        csrf_token: user.csrf_token,
        expires_at: user.expires_at,
    }))
}
