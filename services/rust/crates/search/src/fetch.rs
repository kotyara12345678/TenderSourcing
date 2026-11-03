//! Fetch backends: Keenable (default) and a built-in direct crawler with
//! SSRF protections for `FETCH_BACKEND=direct`.
//!
//! The direct backend never follows redirects automatically: every hop is
//! re-validated against [`assert_public_url`] (scheme, credentials, host
//! blocklist, resolved IP ranges) before the next request is sent, and the
//! redirect budget is `FETCH_MAX_REDIRECTS`. A known limitation of
//! check-then-connect is the DNS-rebinding window (the address is resolved
//! again by the connector); it is documented in `docs/SECURITY.md`.

use std::sync::Arc;
use std::time::Duration;

use reqwest::redirect::Policy;
use tender_common::config::Config;
use tender_common::error::{AppError, AppResult};

use crate::client::KeenableClient;
use crate::tender::search::v1::FetchRequest;

#[derive(Debug, Clone)]
pub struct FetchedPage {
    /// Final URL after redirects (direct backend) or the URL reported by
    /// the provider (Keenable backend).
    pub url: String,
    pub title: String,
    pub author: String,
    /// Extracted text/markdown, already truncated to the configured limit.
    pub content: String,
    /// Unix seconds, 0 when unknown.
    pub published_at_unix: i64,
    /// HTTP status of the successful retrieval (200 for the provider path).
    pub http_status: i32,
    /// Provider-reported credits for this fetch (0 when absent/unbilled).
    pub credits: i64,
}

pub enum FetchBackend {
    Keenable(Arc<KeenableClient>),
    Direct(DirectFetcher),
}

impl FetchBackend {
    pub fn from_config(cfg: &Config, keenable: Arc<KeenableClient>) -> AppResult<Self> {
        match cfg.fetch_backend.as_str() {
            "direct" => Ok(FetchBackend::Direct(DirectFetcher::new(cfg)?)),
            _ => Ok(FetchBackend::Keenable(keenable)),
        }
    }

    pub async fn fetch(&self, url: &str, max_chars: usize, live: bool) -> AppResult<FetchedPage> {
        match self {
            FetchBackend::Keenable(client) => {
                let response = client
                    .fetch(&FetchRequest {
                        url: url.to_string(),
                        live,
                        max_chars: max_chars.min(i32::MAX as usize) as i32,
                        prompt: String::new(),
                    })
                    .await?;
                Ok(FetchedPage {
                    url: response.url,
                    title: response.title,
                    author: response.author,
                    content: truncate_chars(&response.content, max_chars),
                    published_at_unix: response.published_at_unix,
                    http_status: 200,
                    credits: response.usage.as_ref().map_or(0, |usage| usage.credits),
                })
            }
            FetchBackend::Direct(direct) => {
                let mut page = direct.fetch(url).await?;
                page.content = truncate_chars(&page.content, max_chars);
                Ok(page)
            }
        }
    }

    /// Provider name recorded in `sources.provider` (matches the DB CHECK).
    pub fn provider_name(&self) -> &'static str {
        match self {
            FetchBackend::Keenable(_) => "keenable",
            FetchBackend::Direct(_) => "direct",
        }
    }
}

/// Character-boundary-safe truncation; never splits a UTF-8 codepoint.
pub fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    match text.char_indices().nth(max) {
        Some((end, _)) => text[..end].to_string(),
        None => text.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Direct backend
// ---------------------------------------------------------------------------

pub struct DirectFetcher {
    http: reqwest::Client,
    timeout: Duration,
    max_redirects: u8,
}

impl DirectFetcher {
    pub fn new(cfg: &Config) -> AppResult<Self> {
        let http = reqwest::Client::builder()
            .redirect(Policy::none()) // every hop is validated manually
            .timeout(Duration::from_secs(cfg.fetch_timeout_seconds))
            .connect_timeout(Duration::from_secs(10))
            .user_agent(concat!("TenderSourcing/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|err| AppError::internal(format!("direct client build: {err}")))?;
        Ok(Self {
            http,
            timeout: Duration::from_secs(cfg.fetch_timeout_seconds),
            max_redirects: cfg.fetch_max_redirects,
        })
    }

    pub async fn fetch(&self, url: &str) -> AppResult<FetchedPage> {
        let mut current = url.to_string();
        for _ in 0..=self.max_redirects {
            assert_public_url(&current).await?;
            let response = tokio::time::timeout(self.timeout, self.http.get(&current).send())
                .await
                .map_err(|_| AppError::Timeout(format!("direct fetch timed out: {current}")))?
                .map_err(|err| AppError::Upstream(format!("direct fetch: {err}")))?;

            let status = response.status();
            if status.is_redirection() {
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .ok_or_else(|| AppError::Upstream("redirect without Location header".into()))?;
                let base = reqwest::Url::parse(&current)
                    .map_err(|err| AppError::Upstream(format!("bad url: {err}")))?;
                current = base
                    .join(location)
                    .map_err(|err| AppError::Upstream(format!("bad redirect: {err}")))?
                    .to_string();
                continue;
            }
            if !status.is_success() {
                return Err(AppError::Upstream(format!(
                    "direct fetch HTTP {}",
                    status.as_u16()
                )));
            }

            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or("")
                .to_ascii_lowercase();
            if !(content_type.contains("text/html")
                || content_type.contains("text/plain")
                || content_type.contains("application/xhtml"))
            {
                return Err(AppError::Upstream(format!(
                    "unsupported content type '{content_type}'"
                )));
            }

            let body = response.text().await.unwrap_or_default();
            let (title, text) = html_to_text(&body);
            return Ok(FetchedPage {
                url: current,
                title,
                author: String::new(),
                content: text,
                published_at_unix: 0,
                http_status: i32::from(status.as_u16()),
                credits: 0,
            });
        }
        Err(AppError::Upstream(format!(
            "too many redirects (> {}) fetching {url}",
            self.max_redirects
        )))
    }
}

// ---------------------------------------------------------------------------
// SSRF guard
// ---------------------------------------------------------------------------

/// Reject non-public targets before any bytes are sent.
pub async fn assert_public_url(url: &str) -> AppResult<()> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|err| AppError::Upstream(format!("invalid url '{url}': {err}")))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => {
            return Err(AppError::Upstream(format!(
                "refusing scheme '{other}' in {url}"
            )))
        }
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(AppError::Upstream(format!(
            "refusing url with credentials: {url}"
        )));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| AppError::Upstream(format!("url has no host: {url}")))?;
    let port = parsed.port_or_known_default().unwrap_or(80);

    if is_blocked_hostname(host) {
        return Err(AppError::Upstream(format!(
            "refusing blocked host '{host}'"
        )));
    }

    // Resolve once and verify every address; the connector resolves again
    // (documented rebinding window in docs/SECURITY.md).
    let addrs = tokio::net::lookup_host((host, port))
        .await
        .map_err(|err| AppError::Upstream(format!("dns lookup failed for '{host}': {err}")))?;
    let mut saw_addr = false;
    for addr in addrs {
        saw_addr = true;
        if !is_public_ip(addr.ip()) {
            return Err(AppError::Upstream(format!(
                "host '{host}' resolves to non-public address {}",
                addr.ip()
            )));
        }
    }
    if !saw_addr {
        return Err(AppError::Upstream(format!("no addresses for '{host}'")));
    }
    Ok(())
}

fn is_blocked_hostname(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".internal")
        || host.ends_with(".lan")
        || host.ends_with(".home")
        || host.ends_with(".localdomain")
        || host == "metadata.google.internal"
        || host == "metadata"
        || host == "0.0.0.0"
}

/// True when the address may be reached from the fetcher.
pub fn is_public_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => is_public_v4(v4),
        std::net::IpAddr::V6(v6) => {
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return is_public_v4(mapped);
            }
            let segments = v6.segments();
            let unique_local = segments[0] & 0xfe00 == 0xfc00; // fc00::/7
            let link_local = segments[0] & 0xffc0 == 0xfe80; // fe80::/10
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || unique_local
                || link_local)
        }
    }
}

fn is_public_v4(v4: std::net::Ipv4Addr) -> bool {
    let octets = v4.octets();
    let cgnat = octets[0] == 100 && (64..=127).contains(&octets[1]); // 100.64/10
    !(v4.is_private()
        || v4.is_loopback()
        || v4.is_link_local()
        || v4.is_broadcast()
        || v4.is_documentation()
        || v4.is_unspecified()
        || v4.is_multicast()
        || cgnat
        || octets[0] == 0)
}

// ---------------------------------------------------------------------------
// Minimal HTML → text extraction (no external scraper dependency)
// ---------------------------------------------------------------------------

/// Remove `<tag ...>...</tag>` regions case-insensitively. An unclosed tag
/// drops everything from its opening (conservative: better to lose tail
/// text than to leak script contents into extracted page text).
fn strip_tag_blocks(source: &str, tag: &str) -> String {
    let lower = source.to_ascii_lowercase(); // ascii-only: byte indices align
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut out = String::with_capacity(source.len());
    let mut cursor = 0usize;
    while let Some(rel) = lower[cursor..].find(&open) {
        let start = cursor + rel;
        let name_end = start + 1 + tag.len();
        let next = lower[name_end..].chars().next();
        if !matches!(
            next,
            Some('>') | Some(' ') | Some('\t') | Some('\n') | Some('\r') | Some('/')
        ) {
            // Not actually a tag (e.g. "<scripting"), keep scanning past it.
            out.push_str(&source[cursor..name_end]);
            cursor = name_end;
            continue;
        }
        out.push_str(&source[cursor..start]);
        match lower[name_end..].find(&close) {
            Some(rel_close) => cursor = name_end + rel_close + close.len(),
            None => cursor = source.len(), // unclosed: drop the rest
        }
    }
    out.push_str(&source[cursor..]);
    out
}

/// Remove `<!-- ... -->` comment regions.
fn strip_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut cursor = 0usize;
    while let Some(rel) = source[cursor..].find("<!--") {
        let start = cursor + rel;
        out.push_str(&source[cursor..start]);
        match source[start + 4..].find("-->") {
            Some(rel_end) => cursor = start + 4 + rel_end + 3,
            None => cursor = source.len(),
        }
    }
    out.push_str(&source[cursor..]);
    out
}

fn decode_entities(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
        .replace("&#160;", " ")
}

/// Strip tags and scripts/styles, decode the common entities, collapse
/// whitespace. Returns `(title, text)`.
pub fn html_to_text(html: &str) -> (String, String) {
    let mut source = html.to_string();
    source = strip_comments(&source);
    source = strip_tag_blocks(&source, "script");
    source = strip_tag_blocks(&source, "style");
    source = strip_tag_blocks(&source, "noscript");

    let title = {
        let lower = source.to_ascii_lowercase();
        lower
            .find("<title")
            .and_then(|start| {
                let open_rel = lower[start..].find('>')?;
                let open = start + open_rel + 1;
                let end_rel = lower[open..].find("</title>")?;
                Some(decode_entities(&source[open..open + end_rel]))
            })
            .unwrap_or_default()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };

    // Block-level tags become newlines so paragraphs stay separated.
    let with_breaks = source
        .replace("</p>", "\n")
        .replace("</div>", "\n")
        .replace("</br>", "\n")
        .replace("<br/>", "\n")
        .replace("<br>", "\n")
        .replace("</li>", "\n")
        .replace("</h1>", "\n")
        .replace("</h2>", "\n")
        .replace("</h3>", "\n")
        .replace("</tr>", "\n");

    let mut text = String::with_capacity(with_breaks.len());
    let mut in_tag = false;
    for ch in with_breaks.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => text.push(ch),
            _ => {}
        }
    }
    let text = decode_entities(&text);
    let text = text
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    (title, text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    #[test]
    fn public_ip_ranges() {
        let public = [
            "93.184.216.34",
            "8.8.8.8",
            "172.15.0.1", // just outside 172.16/12
            "172.32.0.1", // just outside 172.16/12
            "2606:4700::1111",
        ];
        for ip in public {
            assert!(is_public_ip(ip.parse().unwrap()), "{ip} must be public");
        }

        let blocked = [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.5.5",
            "192.168.1.1",
            "169.254.169.254", // cloud metadata
            "100.64.0.1",      // CGNAT
            "0.0.0.0",
            "224.0.0.1",
            "192.0.2.1", // documentation range
            "::1",
            "fe80::1",
            "fc00::1",
            "::",
            "::ffff:127.0.0.1", // v4-mapped loopback
        ];
        for ip in blocked {
            let parsed: IpAddr = ip.parse().unwrap();
            assert!(!is_public_ip(parsed), "{ip} must be blocked");
        }
    }

    #[test]
    fn hostname_blocklist() {
        assert!(is_blocked_hostname("localhost"));
        assert!(is_blocked_hostname("db.internal"));
        assert!(is_blocked_hostname("printer.local"));
        assert!(is_blocked_hostname("METADATA.GOOGLE.INTERNAL"));
        assert!(!is_blocked_hostname("example.com"));
        assert!(!is_blocked_hostname("keenable.ai"));
    }

    #[tokio::test]
    async fn assert_public_url_rejects_schemes_and_credentials() {
        assert!(assert_public_url("ftp://example.com/x").await.is_err());
        assert!(assert_public_url("http://user:pass@example.com/")
            .await
            .is_err());
        assert!(assert_public_url("http://localhost:8080/").await.is_err());
        assert!(assert_public_url("http://169.254.169.254/latest/meta-data")
            .await
            .is_err());
        assert!(assert_public_url("not a url").await.is_err());
    }

    #[test]
    fn html_extraction_keeps_readable_text() {
        let html = r#"
            <html><head><title>  Widget Specs  </title>
            <style>body { color: red }</style>
            <script>alert("x")</script></head>
            <body><h1>Widget</h1><p>First paragraph &amp; details.</p>
            <p>Second   paragraph</p></body></html>"#;
        let (title, text) = html_to_text(html);
        assert_eq!(title, "Widget Specs");
        assert!(text.contains("First paragraph & details."), "{text}");
        assert!(text.contains("Second paragraph"), "{text}");
        assert!(!text.contains("alert"), "{text}");
        assert!(!text.contains("color: red"), "{text}");
    }

    #[test]
    fn unclosed_script_drops_tail_instead_of_leaking_it() {
        let (title, text) = html_to_text("<title>T</title><p>keep</p><script>evil()");
        assert_eq!(title, "T");
        assert!(text.contains("keep"), "{text}");
        assert!(!text.contains("evil"), "{text}");
    }

    #[test]
    fn truncate_never_splits_utf8() {
        assert_eq!(truncate_chars("привет", 3), "при");
        assert_eq!(truncate_chars("ok", 10), "ok");
        assert_eq!(truncate_chars("абв", 2), "аб");
    }

    #[test]
    fn direct_url_parses_and_joins() {
        let base = reqwest::Url::parse("https://example.com/a/b").unwrap();
        assert_eq!(base.join("/x").unwrap().as_str(), "https://example.com/x");
        assert_eq!(base.join("c").unwrap().as_str(), "https://example.com/a/c");
    }

    #[test]
    fn v6_and_v4_literal_helpers() {
        let v6: IpAddr = IpAddr::V6(Ipv6Addr::LOCALHOST);
        assert!(!is_public_ip(v6));
        let v4: IpAddr = IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1));
        assert!(is_public_ip(v4));
    }
}
