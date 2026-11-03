//! Redis Streams queues with consumer-group semantics (at-least-once).
//!
//! Every job and event payload is a serialized protobuf message — the
//! `.proto` files are the single source of truth for payload contracts.
//! An entry carries a `payload` field plus an optional `kind` field that
//! lets consumers distinguish message types on a shared stream (used by
//! `events:ai`, which carries both requirements and products notices).
//!
//! Delivery is at-least-once: handlers must deduplicate by `task_id`
//! (documented in `common.proto JobEnvelope`).

use redis::{aio::ConnectionManager, cmd, Value};

use crate::error::{AppError, AppResult};

// ---------------------------------------------------------------------------
// Stream and group names (single place where topology lives).
// ---------------------------------------------------------------------------

/// Job streams (producers XADD, workers XREADGROUP).
pub mod jobs {
    pub const DOCUMENTS: &str = "jobs:documents";
    pub const AI_REQUIREMENTS: &str = "jobs:ai.requirements";
    pub const AI_PRODUCTS: &str = "jobs:ai.products";
    pub const SEARCH: &str = "jobs:search";
    pub const REPORTS: &str = "jobs:reports";
    /// Control messages from the API to the orchestrator: a `JobEnvelope`
    /// with `metadata.action` = `start_run` | `cancel` | `regenerate_report`.
    pub const CONTROL: &str = "jobs:control";
}

/// Event streams consumed by the orchestrator.
pub mod events {
    pub const DOCUMENTS: &str = "events:documents";
    pub const AI: &str = "events:ai";
    pub const SEARCH: &str = "events:search";
    pub const REPORTS: &str = "events:reports";
}

/// Pub/Sub channel carrying serialized `ProgressEvent` protobuf bytes;
/// the API decodes them and fans JSON out to browser WebSockets.
pub const PROGRESS_CHANNEL: &str = "events:progress";

/// Consumer groups.
pub mod groups {
    pub const DOC_WORKERS: &str = "doc-workers";
    pub const AI_WORKERS: &str = "ai-workers";
    pub const SEARCH_WORKERS: &str = "search-workers";
    pub const REPORT_WORKERS: &str = "report-workers";
    pub const ORCHESTRATOR: &str = "orchestrator";
}

/// Field names inside a stream entry.
pub const FIELD_PAYLOAD: &str = "payload";
pub const FIELD_KIND: &str = "kind";

/// Consumer identity: stable per host + process, unique across replicas.
pub fn consumer_identity() -> String {
    let host = std::env::var("HOSTNAME").unwrap_or_else(|_| "local".to_string());
    format!("{host}-{}", std::process::id())
}

/// One message read from a stream.
#[derive(Debug, Clone, PartialEq)]
pub struct StreamMessage {
    pub id: String,
    pub kind: String,
    pub payload: Vec<u8>,
}

/// Multiplexed, auto-reconnecting handle. Cheap to clone.
#[derive(Clone)]
pub struct Streams {
    manager: ConnectionManager,
}

impl Streams {
    pub async fn connect(redis_url: &str) -> AppResult<Self> {
        let client = redis::Client::open(redis_url)?;
        let manager = ConnectionManager::new(client).await?;
        Ok(Self { manager })
    }

    /// Create the consumer group from `0` (whole stream) if absent, so no
    /// message published before the first worker boot is ever skipped.
    /// An existing group is left untouched (BUSYGROUP is expected).
    pub async fn ensure_group(&self, stream: &str, group: &str) -> AppResult<()> {
        let mut con = self.manager.clone();
        let result = cmd("XGROUP")
            .arg("CREATE")
            .arg(stream)
            .arg(group)
            .arg("0")
            .arg("MKSTREAM")
            .query_async::<Value>(&mut con)
            .await;
        match result {
            Ok(_) => Ok(()),
            Err(err) if err.to_string().contains("BUSYGROUP") => Ok(()),
            Err(err) => Err(err.into()),
        }
    }

    /// Append a payload-only entry (no kind discriminator).
    pub async fn xadd(&self, stream: &str, payload: &[u8]) -> AppResult<String> {
        self.xadd_kind(stream, "", payload).await
    }

    /// Append an entry with a `kind` discriminator (e.g. "requirements").
    pub async fn xadd_kind(&self, stream: &str, kind: &str, payload: &[u8]) -> AppResult<String> {
        let mut con = self.manager.clone();
        let id: String = cmd("XADD")
            .arg(stream)
            .arg("*")
            .arg(FIELD_PAYLOAD)
            .arg(payload)
            .arg(FIELD_KIND)
            .arg(kind)
            .query_async(&mut con)
            .await?;
        Ok(id)
    }

    /// Read up to `count` new messages for `group`, blocking up to `block_ms`
    /// (0 = block forever; callers should pass a finite value so shutdown
    /// stays responsive). Returns an empty vector on block timeout.
    pub async fn read(
        &self,
        stream: &str,
        group: &str,
        consumer: &str,
        count: usize,
        block_ms: u64,
    ) -> AppResult<Vec<StreamMessage>> {
        let mut con = self.manager.clone();
        let value = cmd("XREADGROUP")
            .arg("GROUP")
            .arg(group)
            .arg(consumer)
            .arg("COUNT")
            .arg(count)
            .arg("BLOCK")
            .arg(block_ms)
            .arg("STREAMS")
            .arg(stream)
            .arg(">")
            .query_async::<Value>(&mut con)
            .await?;
        parse_read_response(value, stream)
    }

    /// Claim messages abandoned by a dead consumer (idle > `min_idle_ms`).
    /// This is the crash-recovery path for at-least-once delivery.
    pub async fn autoclaim(
        &self,
        stream: &str,
        group: &str,
        consumer: &str,
        min_idle_ms: u64,
        count: usize,
    ) -> AppResult<Vec<StreamMessage>> {
        let mut con = self.manager.clone();
        let value = cmd("XAUTOCLAIM")
            .arg(stream)
            .arg(group)
            .arg(consumer)
            .arg(min_idle_ms)
            .arg("0")
            .arg("COUNT")
            .arg(count)
            .query_async::<Value>(&mut con)
            .await?;
        parse_autoclaim_response(value)
    }

    /// Acknowledge processed messages; returns how many were acked.
    pub async fn ack(&self, stream: &str, group: &str, ids: &[String]) -> AppResult<u64> {
        if ids.is_empty() {
            return Ok(0);
        }
        let mut con = self.manager.clone();
        let mut command = cmd("XACK");
        command.arg(stream).arg(group);
        for id in ids {
            command.arg(id);
        }
        let acked: i64 = command.query_async(&mut con).await?;
        Ok(acked.max(0) as u64)
    }

    /// Publish protobuf bytes to a Pub/Sub channel; returns receivers.
    pub async fn publish(&self, channel: &str, payload: &[u8]) -> AppResult<u64> {
        let mut con = self.manager.clone();
        let receivers: i64 = cmd("PUBLISH")
            .arg(channel)
            .arg(payload)
            .query_async(&mut con)
            .await?;
        Ok(receivers.max(0) as u64)
    }

    /// SET key value NX EX ttl_secs — idempotency guard; true when acquired.
    pub async fn set_nx(&self, key: &str, ttl_secs: u64) -> AppResult<bool> {
        let mut con = self.manager.clone();
        let value: Value = cmd("SET")
            .arg(key)
            .arg("1")
            .arg("NX")
            .arg("EX")
            .arg(ttl_secs)
            .query_async(&mut con)
            .await?;
        Ok(matches!(value, Value::Okay))
    }

    /// Release an idempotency key so a failed attempt can be retried.
    pub async fn release(&self, key: &str) -> AppResult<()> {
        let mut con = self.manager.clone();
        let _: i64 = cmd("DEL").arg(key).query_async(&mut con).await?;
        Ok(())
    }

    pub async fn ping(&self) -> AppResult<()> {
        let mut con = self.manager.clone();
        let _: String = cmd("PING").query_async(&mut con).await?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Response parsing — pure functions, unit tested without a server.
// ---------------------------------------------------------------------------

fn bulk_to_string(value: &Value) -> Option<String> {
    match value {
        Value::BulkString(bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
        Value::SimpleString(text) => Some(text.clone()),
        _ => None,
    }
}

fn shape_error(what: &str, value: &Value) -> AppError {
    AppError::internal(format!("unexpected {what} shape: {value:?}"))
}

/// XREADGROUP → `[[stream, [[id, [field, value, ...]], ...]], ...]`,
/// `Nil` on BLOCK timeout.
fn parse_read_response(value: Value, expected_stream: &str) -> AppResult<Vec<StreamMessage>> {
    match value {
        Value::Nil => Ok(Vec::new()),
        Value::Array(streams) => {
            let mut messages = Vec::new();
            for stream_value in streams {
                let pair = match stream_value {
                    Value::Array(pair) => pair,
                    other => return Err(shape_error("stream element", &other)),
                };
                if pair.len() < 2 {
                    continue;
                }
                let name = bulk_to_string(&pair[0]).unwrap_or_default();
                if name != expected_stream {
                    return Err(AppError::internal(format!(
                        "XREADGROUP returned stream '{name}', expected '{expected_stream}'"
                    )));
                }
                let entries = match &pair[1] {
                    Value::Array(entries) => entries,
                    other => return Err(shape_error("entries", other)),
                };
                for entry in entries {
                    messages.push(parse_entry(entry)?);
                }
            }
            Ok(messages)
        }
        other => Err(shape_error("XREADGROUP response", &other)),
    }
}

/// XAUTOCLAIM → `[next_start, [[id, [f, v], ...], ...], [deleted ids...]]`.
fn parse_autoclaim_response(value: Value) -> AppResult<Vec<StreamMessage>> {
    match value {
        Value::Array(parts) => {
            let mut messages = Vec::new();
            if let Some(Value::Array(entries)) = parts.get(1) {
                for entry in entries {
                    messages.push(parse_entry(entry)?);
                }
            }
            Ok(messages)
        }
        other => Err(shape_error("XAUTOCLAIM response", &other)),
    }
}

/// A single entry: `[id, [field, value, ...]]`.
fn parse_entry(entry: &Value) -> AppResult<StreamMessage> {
    let fields = match entry {
        Value::Array(fields) => fields,
        other => return Err(shape_error("entry", other)),
    };
    if fields.len() < 2 {
        return Err(shape_error("entry (need id + field map)", entry));
    }
    let id = bulk_to_string(&fields[0]).ok_or_else(|| shape_error("entry id", &fields[0]))?;
    let pairs = match &fields[1] {
        Value::Array(pairs) => pairs,
        other => return Err(shape_error("entry field map", other)),
    };

    let mut kind = String::new();
    let mut payload: Option<Vec<u8>> = None;
    let mut iter = pairs.iter();
    while let (Some(key), Some(value)) = (iter.next(), iter.next()) {
        match bulk_to_string(key).as_deref() {
            Some(FIELD_KIND) => kind = bulk_to_string(value).unwrap_or_default(),
            Some(FIELD_PAYLOAD) => match value {
                Value::BulkString(bytes) => payload = Some(bytes.clone()),
                other => return Err(shape_error("payload field", other)),
            },
            // Unknown fields are tolerated for forward compatibility.
            _ => {}
        }
    }
    let payload = payload.ok_or_else(|| {
        AppError::internal(format!("stream entry {id} has no '{FIELD_PAYLOAD}' field"))
    })?;
    Ok(StreamMessage { id, kind, payload })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, fields: Vec<(&str, Value)>) -> Value {
        let mut flat = Vec::new();
        for (key, value) in fields {
            flat.push(Value::BulkString(key.as_bytes().to_vec()));
            flat.push(value);
        }
        Value::Array(vec![
            Value::BulkString(id.as_bytes().to_vec()),
            Value::Array(flat),
        ])
    }

    #[test]
    fn read_nil_means_block_timeout() {
        assert!(parse_read_response(Value::Nil, "jobs:search")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn read_parses_payload_and_kind() {
        let response = Value::Array(vec![Value::Array(vec![
            Value::BulkString(b"events:ai".to_vec()),
            Value::Array(vec![entry(
                "1700-0",
                vec![
                    ("payload", Value::BulkString(b"\x08\x01".to_vec())),
                    ("kind", Value::BulkString(b"requirements".to_vec())),
                ],
            )]),
        ])]);
        let messages = parse_read_response(response, "events:ai").unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].id, "1700-0");
        assert_eq!(messages[0].kind, "requirements");
        assert_eq!(messages[0].payload, b"\x08\x01");
    }

    #[test]
    fn read_rejects_wrong_stream_name() {
        let response = Value::Array(vec![Value::Array(vec![
            Value::BulkString(b"jobs:other".to_vec()),
            Value::Array(vec![]),
        ])]);
        assert!(parse_read_response(response, "jobs:search").is_err());
    }

    #[test]
    fn entry_without_payload_is_an_error() {
        let bad = entry("1-0", vec![("kind", Value::BulkString(b"x".to_vec()))]);
        assert!(parse_entry(&bad).is_err());
    }

    #[test]
    fn unknown_fields_are_tolerated() {
        let good = entry(
            "2-0",
            vec![
                ("future_field", Value::BulkString(b"whatever".to_vec())),
                ("payload", Value::BulkString(b"data".to_vec())),
            ],
        );
        let message = parse_entry(&good).unwrap();
        assert_eq!(message.payload, b"data");
        assert_eq!(message.kind, "");
    }

    #[test]
    fn autoclaim_parses_claimed_entries_and_empty_sets() {
        let response = Value::Array(vec![
            Value::BulkString(b"1900-0".to_vec()),
            Value::Array(vec![entry(
                "1800-0",
                vec![("payload", Value::BulkString(b"p".to_vec()))],
            )]),
            Value::Array(vec![]),
        ]);
        let messages = parse_autoclaim_response(response).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].id, "1800-0");

        let empty = Value::Array(vec![
            Value::BulkString(b"0-0".to_vec()),
            Value::Array(vec![]),
            Value::Array(vec![]),
        ]);
        assert!(parse_autoclaim_response(empty).unwrap().is_empty());
    }
}
