//! Keenable HTTP client — https://docs.keenable.ai
//!
//! * `POST {base}/v1/search` with `X-API-Key`, JSON body per the API
//!   reference (`query`, `mode`, `site`, date filters, `snippet_max_length`
//!   180..10000, `max_results` 1..50).
//! * `GET {base}/v1/fetch?url=...&max_chars=...&live=...&prompt=...`.
//!
//! Responses carry `X-RateLimit-*` headers and a 429 adds `Retry-After`;
//! the shared [`RateLimiter`] is pushed past the provider deadline before
//! any retry. Server errors are retried with linear backoff, everything else
//! fails fast with an `Upstream` error whose message never contains the key.

use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;

use tender_common::config::Config;
use tender_common::error::{AppError, AppResult};

use crate::ratelimit::RateLimiter;
use crate::tender::search::v1::{
    FetchRequest, FetchResponse, SearchRequest, SearchResponse, SearchResult, Usage,
};

/// Upper bound on HTTP attempts per logical call (429 and 5xx are retried).
const MAX_ATTEMPTS: u32 = 3;

#[derive(Debug, Deserialize)]
struct SearchResponseBody {
    #[serde(default)]
    query: String,
    #[serde(default)]
    mode: String,
    #[serde(default)]
    results: Vec<SearchResultBody>,
    #[serde(default)]
    usage: Option<UsageBody>,
}

#[derive(Debug, Deserialize)]
struct SearchResultBody {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    /// Deprecated upstream: always empty, kept as a fallback just in case.
    #[serde(default)]
    description: String,
    #[serde(default)]
    snippet: String,
    #[serde(default)]
    published_at: String,
    #[serde(default)]
    acquired_at: String,
}

#[derive(Debug, Deserialize)]
struct UsageBody {
    #[serde(default)]
    sku: String,
    #[serde(default)]
    credits: i64,
    #[serde(default)]
    paid: bool,
}

#[derive(Debug, Deserialize)]
struct FetchResponseBody {
    #[serde(default)]
    url: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    author: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    published_at: i64,
    #[serde(default)]
    usage: Option<UsageBody>,
}

impl From<UsageBody> for Usage {
    fn from(body: UsageBody) -> Self {
        Usage {
            sku: body.sku,
            credits: body.credits,
            paid: body.paid,
        }
    }
}

pub struct KeenableClient {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    limiter: Arc<RateLimiter>,
    default_mode: String,
    probe_timeout: Duration,
}

impl KeenableClient {
    pub fn new(cfg: &Config) -> AppResult<Self> {
        let api_key = cfg.require_keenable_api_key()?.to_string();
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(cfg.fetch_timeout_seconds.max(10)))
            .connect_timeout(Duration::from_secs(10))
            .user_agent(concat!("TenderSourcing/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|err| AppError::internal(format!("http client build: {err}")))?;
        Ok(Self {
            http,
            base_url: cfg.keenable_base_url.trim_end_matches('/').to_string(),
            api_key,
            limiter: Arc::new(RateLimiter::new(cfg.keenable_max_rps)),
            default_mode: cfg.keenable_search_mode.clone(),
            probe_timeout: Duration::from_secs(5),
        })
    }

    pub fn limiter(&self) -> Arc<RateLimiter> {
        Arc::clone(&self.limiter)
    }

    /// Execute one HTTP call with rate limiting, 429/Retry-After handling
    /// and 5xx retries. `make` rebuilds the request on every attempt.
    async fn send_with_retry<F>(&self, make: F) -> AppResult<reqwest::Response>
    where
        F: Fn() -> reqwest::RequestBuilder,
    {
        let mut attempt = 0u32;
        loop {
            attempt += 1;
            self.limiter.acquire().await;
            let response = make()
                .send()
                .await
                .map_err(|err| AppError::Upstream(format!("keenable transport: {err}")))?;
            let status = response.status();

            if status.as_u16() == 429 {
                let retry_after = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse::<u64>().ok())
                    .unwrap_or(5)
                    .min(60);
                self.limiter.penalize(retry_after);
                if attempt >= MAX_ATTEMPTS {
                    return Err(AppError::RateLimited(retry_after));
                }
                continue;
            }

            if status.is_server_error() && attempt < MAX_ATTEMPTS {
                tokio::time::sleep(Duration::from_secs(u64::from(attempt))).await;
                continue;
            }

            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                let snippet: String = body.chars().take(500).collect();
                return Err(AppError::Upstream(format!(
                    "keenable HTTP {}: {snippet}",
                    status.as_u16()
                )));
            }
            return Ok(response);
        }
    }

    pub async fn search(&self, request: &SearchRequest) -> AppResult<SearchResponse> {
        if request.query.trim().is_empty() {
            return Err(AppError::Validation("search query is required".into()));
        }
        if request.max_results > 50 || request.snippet_max_length > 10_000 {
            return Err(AppError::Validation(
                "max_results must be <= 50 and snippet_max_length <= 10000".into(),
            ));
        }

        let mode = if request.mode.is_empty() {
            self.default_mode.clone()
        } else {
            request.mode.clone()
        };

        let mut body = serde_json::Map::new();
        body.insert("query".into(), request.query.clone().into());
        body.insert("mode".into(), mode.into());
        if !request.site.is_empty() {
            body.insert("site".into(), request.site.clone().into());
        }
        if !request.published_after.is_empty() {
            body.insert(
                "published_after".into(),
                request.published_after.clone().into(),
            );
        }
        if !request.published_before.is_empty() {
            body.insert(
                "published_before".into(),
                request.published_before.clone().into(),
            );
        }
        if request.snippet_max_length >= 180 {
            body.insert(
                "snippet_max_length".into(),
                request.snippet_max_length.into(),
            );
        }
        if request.max_results >= 1 {
            body.insert("max_results".into(), request.max_results.into());
        }
        let body = serde_json::Value::Object(body);

        let url = format!("{}/v1/search", self.base_url);
        let response = self
            .send_with_retry(|| {
                self.http
                    .post(&url)
                    .header("X-API-Key", self.api_key.as_str())
                    .json(&body)
            })
            .await?;

        let parsed: SearchResponseBody = response
            .json()
            .await
            .map_err(|err| AppError::Upstream(format!("keenable search decode: {err}")))?;

        let results = parsed
            .results
            .into_iter()
            .map(|result| SearchResult {
                title: result.title,
                url: result.url,
                // `description` is deprecated upstream and normally empty;
                // `snippet` is the long excerpt we store.
                snippet: if result.snippet.is_empty() {
                    result.description
                } else {
                    result.snippet
                },
                published_at: result.published_at,
                acquired_at: result.acquired_at,
            })
            .collect();

        Ok(SearchResponse {
            query: parsed.query,
            mode: parsed.mode,
            results,
            usage: parsed.usage.map(Usage::from),
        })
    }

    pub async fn fetch(&self, request: &FetchRequest) -> AppResult<FetchResponse> {
        if request.url.trim().is_empty() {
            return Err(AppError::Validation("fetch url is required".into()));
        }
        if request.prompt.chars().count() > 2_000 {
            return Err(AppError::Validation("prompt must be <= 2000 chars".into()));
        }

        let url = format!("{}/v1/fetch", self.base_url);
        let target = request.url.clone();
        let max_chars = request.max_chars;
        let live = request.live;
        let prompt = request.prompt.clone();
        let response = self
            .send_with_retry(|| {
                let mut builder = self
                    .http
                    .get(&url)
                    .header("X-API-Key", self.api_key.as_str())
                    .query(&[("url", target.as_str())]);
                if max_chars > 0 {
                    builder = builder.query(&[("max_chars", max_chars)]);
                }
                if live {
                    builder = builder.query(&[("live", "true")]);
                }
                if !prompt.is_empty() {
                    builder = builder.query(&[("prompt", prompt.as_str())]);
                }
                builder
            })
            .await?;

        let parsed: FetchResponseBody = response
            .json()
            .await
            .map_err(|err| AppError::Upstream(format!("keenable fetch decode: {err}")))?;

        Ok(FetchResponse {
            url: parsed.url,
            title: parsed.title,
            author: parsed.author,
            content: parsed.content,
            published_at_unix: parsed.published_at,
            usage: parsed.usage.map(Usage::from),
        })
    }

    /// Reachability probe used by the gRPC `Health` method: the public fetch
    /// twin needs no key and consumes no credits, so it reports real upstream
    /// reachability instead of "config looks fine".
    pub async fn probe_public(&self) -> AppResult<()> {
        let url = format!("{}/v1/fetch/public", self.base_url);
        self.limiter.acquire().await;
        let response = tokio::time::timeout(
            self.probe_timeout,
            self.http
                .get(&url)
                .query(&[("url", "https://example.com")])
                .header("X-Keenable-Title", "TenderSourcing health probe")
                .send(),
        )
        .await
        .map_err(|_| AppError::Timeout("keenable probe timed out".into()))?
        .map_err(|err| AppError::Upstream(format!("keenable probe transport: {err}")))?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(AppError::Upstream(format!(
                "keenable probe HTTP {}",
                response.status().as_u16()
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_maps_from_provider_body() {
        let body: UsageBody =
            serde_json::from_str(r#"{"sku":"search.pro","credits":3,"paid":true}"#)
                .expect("valid usage json");
        let usage = Usage::from(body);
        assert_eq!(usage.sku, "search.pro");
        assert_eq!(usage.credits, 3);
        assert!(usage.paid);
    }

    #[test]
    fn search_result_prefers_snippet_over_deprecated_description() {
        // Deserialization-level check of the fallback logic without network.
        let body: SearchResultBody = serde_json::from_str(
            r#"{"title":"t","url":"u","description":"deprecated","snippet":"long"}"#,
        )
        .unwrap();
        let snippet = if body.snippet.is_empty() {
            body.description
        } else {
            body.snippet
        };
        assert_eq!(snippet, "long");

        let body: SearchResultBody =
            serde_json::from_str(r#"{"title":"t","url":"u","description":"d"}"#).unwrap();
        let snippet = if body.snippet.is_empty() {
            body.description
        } else {
            body.snippet
        };
        assert_eq!(snippet, "d");
    }
}
