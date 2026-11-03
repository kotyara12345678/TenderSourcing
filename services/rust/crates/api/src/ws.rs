//! WebSocket progress feed.
//!
//! One Redis Pub/Sub connection per socket (single-operator deployment).
//! The browser authenticates with the same HttpOnly session cookie, receives
//! an initial snapshot, then every `ProgressEvent` for the tender as JSON.

use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::response::Response;
use futures::{SinkExt, StreamExt};
use prost::Message as ProstMessage;
use uuid::Uuid;

use tender_common::error::{AppError, AppResult};
use tender_common::streams;

use crate::auth::SessionUser;
use crate::state::AppState;
use crate::tender::common::v1::ProgressEvent;

const KEEPALIVE_SECONDS: u64 = 30;

pub async fn tender_progress(
    State(state): State<AppState>,
    user: SessionUser,
    Path(tender_id): Path<Uuid>,
    upgrade: WebSocketUpgrade,
) -> AppResult<Response> {
    // Authorize before the upgrade: a browser without a valid session never
    // gets a socket, even though the cookie is attached automatically.
    let _ = user;
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM tenders WHERE id = $1 AND deleted_at IS NULL")
        .bind(tender_id)
        .fetch_optional(&state.pg)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("tender {tender_id}")))?;

    Ok(upgrade.on_upgrade(move |socket| async move {
        if let Err(err) = run_socket(socket, state, tender_id).await {
            tracing::debug!(error = %err, %tender_id, "websocket closed");
        }
    }))
}

async fn run_socket(socket: WebSocket, state: AppState, tender_id: Uuid) -> AppResult<()> {
    let client = redis::Client::open(state.cfg.redis_url.clone())?;
    let mut pubsub = client.get_async_pubsub().await?;
    pubsub.subscribe(streams::PROGRESS_CHANNEL).await?;
    let mut messages = pubsub.on_message();

    let (mut sender, mut receiver) = socket.split();
    let mut keepalive = tokio::time::interval(Duration::from_secs(KEEPALIVE_SECONDS));

    send_snapshot(&state, tender_id, &mut sender).await?;

    loop {
        tokio::select! {
            message = messages.next() => {
                let Some(message) = message else { break };
                let payload: Vec<u8> = match message.get_payload() {
                    Ok(payload) => payload,
                    Err(err) => { tracing::warn!(error = %err, "pubsub payload error"); continue; }
                };
                let Ok(event) = ProgressEvent::decode(payload.as_slice()) else { continue };
                if event.tender_id != tender_id.to_string() { continue }
                let json = serde_json::json!({
                    "type": "progress",
                    "event_id": event.event_id,
                    "event_type": event.event_type,
                    "tender_id": event.tender_id,
                    "task_id": event.task_id,
                    "run_id": event.run_id,
                    "timestamp_unix_ms": event.timestamp_unix_ms,
                    "stage": event.stage,
                    "progress": event.progress,
                    "message": event.message,
                    "level": event.level,
                })
                .to_string();
                if sender.send(Message::Text(json.into())).await.is_err() { break }
            }
            _ = keepalive.tick() => {
                if sender.send(Message::Ping(Vec::new().into())).await.is_err() { break }
            }
            incoming = receiver.next() => {
                match incoming {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

async fn send_snapshot(
    state: &AppState,
    tender_id: Uuid,
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
) -> AppResult<()> {
    let row: Option<(String, i64, i64)> = sqlx::query_as(
        "SELECT t.status,
                (SELECT COUNT(*) FROM components c WHERE c.tender_id = t.id) AS components,
                (SELECT COUNT(*) FROM components c WHERE c.tender_id = t.id AND c.status = 'MATCHED') AS matched
         FROM tenders t WHERE t.id = $1 AND t.deleted_at IS NULL",
    )
    .bind(tender_id)
    .fetch_optional(&state.pg)
    .await?;
    let (status, components, matched) =
        row.ok_or_else(|| AppError::NotFound(format!("tender {tender_id}")))?;
    let snapshot = serde_json::json!({
        "type": "snapshot",
        "tender_id": tender_id,
        "status": status,
        "components": components,
        "matched": matched,
    })
    .to_string();
    sender
        .send(Message::Text(snapshot.into()))
        .await
        .map_err(|err| AppError::internal(format!("websocket send failed: {err}")))
}
