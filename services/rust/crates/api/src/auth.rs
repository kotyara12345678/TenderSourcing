//! Authentication and session handling.
//!
//! * The single operator account is created from `AUTH_USERNAME` /
//!   `AUTH_PASSWORD` on first start. `password_fingerprint` is
//!   `HMAC-SHA256(APP_SECRET, AUTH_PASSWORD)`; when the ENV password changes
//!   the stored hash is re-derived and every existing session is revoked.
//! * Sessions are server-side rows; the HttpOnly cookie carries a random
//!   token and only its SHA-256 is stored, so a database leak does not leak
//!   usable cookies.
//! * Every state-changing request must send `X-CSRF-Token` matching the
//!   session's CSRF token (in addition to `SameSite=Strict`).

use argon2::password_hash::{rand_core::OsRng, PasswordHash, SaltString};
use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use axum::extract::FromRequestParts;
use axum::http::{header, request::Parts, HeaderMap};
use chrono::{DateTime, Duration, Utc};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use tender_common::error::{AppError, AppResult};

use crate::state::AppState;

type HmacSha256 = Hmac<Sha256>;

pub const SESSION_COOKIE: &str = "ts_session";
pub const CSRF_HEADER: &str = "x-csrf-token";

/// Authenticated operator plus the session that proved it.
#[derive(Debug, Clone)]
pub struct SessionUser {
    pub id: Uuid,
    pub username: String,
    pub display_name: String,
    pub session_id: Uuid,
    pub csrf_token: String,
    pub expires_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Password helpers
// ---------------------------------------------------------------------------

pub fn hash_password(password: &str) -> AppResult<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|err| AppError::internal(format!("password hashing failed: {err}")))
}

pub fn verify_password(password: &str, encoded: &str) -> bool {
    PasswordHash::new(encoded)
        .map(|parsed| {
            Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok()
        })
        .unwrap_or(false)
}

/// HMAC-SHA256(APP_SECRET, password) — detects ENV rotation without storing
/// the password anywhere.
pub fn password_fingerprint(secret: &str, password: &str) -> AppResult<String> {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|err| AppError::internal(format!("hmac init failed: {err}")))?;
    mac.update(password.as_bytes());
    Ok(hex::encode(mac.finalize().into_bytes()))
}

/// Create the operator account on first start or re-sync a rotated password.
pub async fn bootstrap_user(state: &AppState) -> AppResult<()> {
    let username = state.cfg.auth_username.trim().to_string();
    let fingerprint = password_fingerprint(&state.cfg.app_secret, &state.cfg.auth_password)?;

    let existing: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT id, password_fingerprint FROM users WHERE username = $1 AND deleted_at IS NULL",
    )
    .bind(&username)
    .fetch_optional(&state.pg)
    .await?;

    match existing {
        None => {
            let hash = hash_password(&state.cfg.auth_password)?;
            sqlx::query(
                "INSERT INTO users (username, password_hash, password_fingerprint, display_name)
                 VALUES ($1, $2, $3, $1)",
            )
            .bind(&username)
            .bind(hash)
            .bind(&fingerprint)
            .execute(&state.pg)
            .await?;
            tracing::info!(%username, "operator account created from environment");
        }
        Some((user_id, stored)) => {
            if stored != fingerprint {
                let hash = hash_password(&state.cfg.auth_password)?;
                sqlx::query(
                    "UPDATE users SET password_hash = $2, password_fingerprint = $3
                     WHERE id = $1",
                )
                .bind(user_id)
                .bind(hash)
                .bind(&fingerprint)
                .execute(&state.pg)
                .await?;
                sqlx::query("UPDATE sessions SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL")
                    .bind(user_id)
                    .execute(&state.pg)
                    .await?;
                tracing::info!(
                    %username,
                    "operator password rotated from environment; sessions revoked"
                );
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

/// 256 bits of entropy from the OS RNG (two UUIDv4 values, 64 hex chars).
pub fn generate_token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

pub fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

pub struct IssuedSession {
    pub token: String,
    pub csrf_token: String,
    pub expires_at: DateTime<Utc>,
}

pub async fn create_session(
    state: &AppState,
    user_id: Uuid,
    user_agent: &str,
    ip: Option<&str>,
) -> AppResult<IssuedSession> {
    let token = generate_token();
    let csrf_token = Uuid::new_v4().simple().to_string();
    let expires_at = Utc::now() + Duration::seconds(state.cfg.auth_session_ttl);
    let user_agent: String = user_agent.chars().take(300).collect();
    let ip: Option<String> = ip
        .map(str::trim)
        .map(|value| {
            value
                .trim_start_matches('[')
                .trim_end_matches(']')
                .to_string()
        })
        .filter(|value| value.parse::<std::net::IpAddr>().is_ok());

    sqlx::query(
        "INSERT INTO sessions (user_id, token_hash, csrf_token, user_agent, ip, expires_at)
         VALUES ($1, $2, $3, $4, $5::inet, $6)",
    )
    .bind(user_id)
    .bind(token_hash(&token))
    .bind(&csrf_token)
    .bind(user_agent)
    .bind(ip)
    .bind(expires_at)
    .execute(&state.pg)
    .await?;

    Ok(IssuedSession {
        token,
        csrf_token,
        expires_at,
    })
}

pub fn session_cookie(state: &AppState, token: &str) -> String {
    let mut cookie = format!(
        "{SESSION_COOKIE}={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age={}",
        state.cfg.auth_session_ttl
    );
    if state.cfg.auth_cookie_secure {
        cookie.push_str("; Secure");
    }
    cookie
}

pub fn clear_cookie(state: &AppState) -> String {
    let mut cookie = format!("{SESSION_COOKIE}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0");
    if state.cfg.auth_cookie_secure {
        cookie.push_str("; Secure");
    }
    cookie
}

fn cookie_value(raw: &str, name: &str) -> Option<String> {
    raw.split(';')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| key.trim() == name)
        .map(|(_, value)| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

impl FromRequestParts<AppState> for SessionUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = parts
            .headers
            .get(header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .and_then(|raw| cookie_value(raw, SESSION_COOKIE))
            .ok_or(AppError::Unauthorized)?;

        let row: Option<(Uuid, String, String, Uuid, String, DateTime<Utc>)> = sqlx::query_as(
            "SELECT u.id, u.username, u.display_name, s.id, s.csrf_token, s.expires_at
             FROM sessions s
             JOIN users u ON u.id = s.user_id
             WHERE s.token_hash = $1
               AND s.revoked_at IS NULL
               AND s.expires_at > now()
               AND u.is_active = TRUE
               AND u.deleted_at IS NULL",
        )
        .bind(token_hash(&token))
        .fetch_optional(&state.pg)
        .await?;

        let (id, username, display_name, session_id, csrf_token, expires_at) =
            row.ok_or(AppError::Unauthorized)?;

        // Sliding activity timestamp, throttled to avoid a write per request.
        let _ = sqlx::query(
            "UPDATE sessions SET last_seen_at = now()
             WHERE id = $1 AND last_seen_at < now() - interval '5 minutes'",
        )
        .bind(session_id)
        .execute(&state.pg)
        .await;

        Ok(SessionUser {
            id,
            username,
            display_name,
            session_id,
            csrf_token,
            expires_at,
        })
    }
}

/// Enforce the CSRF header on every mutating request.
pub fn require_csrf(user: &SessionUser, headers: &HeaderMap) -> AppResult<()> {
    let provided = headers
        .get(CSRF_HEADER)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if !constant_time_eq(provided.as_bytes(), user.csrf_token.as_bytes()) {
        return Err(AppError::Forbidden);
    }
    Ok(())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in left.iter().zip(right.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_hash_round_trip() {
        let hash = hash_password("correct horse battery staple").unwrap();
        assert!(verify_password("correct horse battery staple", &hash));
        assert!(!verify_password("wrong", &hash));
    }

    #[test]
    fn fingerprint_is_secret_bound_and_stable() {
        let first = password_fingerprint("secret-a", "pw").unwrap();
        let second = password_fingerprint("secret-a", "pw").unwrap();
        let other_secret = password_fingerprint("secret-b", "pw").unwrap();
        let other_password = password_fingerprint("secret-a", "pw2").unwrap();
        assert_eq!(first, second);
        assert_ne!(first, other_secret);
        assert_ne!(first, other_password);
        assert_eq!(first.len(), 64);
    }

    #[test]
    fn tokens_are_unique_and_hashed() {
        let a = generate_token();
        let b = generate_token();
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
        assert_eq!(token_hash(&a).len(), 64);
        assert_ne!(token_hash(&a), token_hash(&b));
    }

    #[test]
    fn cookie_parsing() {
        let raw = "other=1; ts_session=abc123; theme=dark";
        assert_eq!(cookie_value(raw, SESSION_COOKIE).as_deref(), Some("abc123"));
        assert_eq!(cookie_value(raw, "missing"), None);
        assert_eq!(cookie_value("ts_session=", SESSION_COOKIE), None);
    }

    #[test]
    fn constant_time_comparison_behaves_like_eq() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(constant_time_eq(b"", b""));
    }
}
