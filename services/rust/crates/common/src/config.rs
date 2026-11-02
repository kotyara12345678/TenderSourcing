//! Centralized environment configuration.
//!
//! Every Rust service constructs [`Config`] at startup. Validation collects
//! *all* problems and reports them as one [`ConfigError`] so an operator can
//! fix the environment in a single pass. Variable names and defaults match
//! `.env.example` exactly.

use std::env;

/// The environment is invalid; contains every problem found, joined by "; ".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError(pub String);

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid configuration: {}", self.0)
    }
}

impl std::error::Error for ConfigError {}

pub type ConfigResult<T> = Result<T, ConfigError>;

/// Full application environment, validated once at startup.
#[derive(Debug, Clone)]
pub struct Config {
    // ---------- application ----------
    pub app_env: String,
    pub app_host: String,
    pub app_port: u16,
    pub app_secret: String,
    pub public_api_origin: String,
    pub cors_origins: Vec<String>,

    // ---------- postgres ----------
    pub database_url: String,
    pub database_max_connections: u32,

    // ---------- redis ----------
    pub redis_url: String,
    pub redis_max_connections: u32,

    // ---------- auth (single operator, no registration) ----------
    pub auth_username: String,
    pub auth_password: String,
    pub auth_secret: String,
    pub auth_session_ttl: i64,
    pub auth_cookie_secure: bool,

    // ---------- Keenable (https://keenable.ai) ----------
    /// `None` when not configured; the search service fails fast if absent.
    pub keenable_api_key: Option<String>,
    pub keenable_base_url: String,
    pub keenable_search_limit: u64,
    pub keenable_fetch_limit: u64,
    pub keenable_max_rps: u64,
    pub keenable_search_mode: String,

    // ---------- fetch engine ----------
    pub fetch_backend: String,
    pub fetch_max_chars: usize,
    pub fetch_timeout_seconds: u64,
    pub fetch_max_redirects: u8,
    pub fetch_max_per_component: u8,

    // ---------- LLM (OpenAI-compatible) ----------
    pub llm_api_key: Option<String>,
    pub llm_base_url: String,
    pub llm_model: String,
    pub llm_fallback_model: String,
    pub llm_timeout: u64,
    pub llm_max_tokens: u32,

    // ---------- OCR ----------
    pub ocr_provider: String,
    pub ocr_api_key: Option<String>,
    pub ocr_base_url: Option<String>,

    // ---------- file storage ----------
    pub storage_provider: String,
    pub storage_path: String,
    pub s3_endpoint: Option<String>,
    pub s3_bucket: Option<String>,
    pub s3_access_key: Option<String>,
    pub s3_secret_key: Option<String>,

    // ---------- business limits ----------
    pub max_file_size: u64,
    pub max_tender_components: u32,
    pub max_search_queries: u32,
    pub max_fetch_requests: u32,
    pub max_llm_tokens_per_tender: u64,
    pub max_tender_duration: u64,

    // ---------- concurrency ----------
    pub worker_concurrency: usize,
    pub search_concurrency: usize,
    pub ai_concurrency: usize,
    pub document_concurrency: usize,

    // ---------- observability ----------
    pub log_level: String,
    pub otel_exporter_otlp_endpoint: Option<String>,
    pub sentry_dsn: Option<String>,
}

// ---------------------------------------------------------------------------
// env helpers: each pushes a problem instead of failing, so `from_env`
// reports everything at once.
// ---------------------------------------------------------------------------

fn raw(key: &str) -> Option<String> {
    env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn with_default(key: &str, default: &str) -> String {
    raw(key).unwrap_or_else(|| default.to_string())
}

fn required(key: &str, errors: &mut Vec<String>) -> String {
    match raw(key) {
        Some(value) => value,
        None => {
            errors.push(format!("{key} is required"));
            String::new()
        }
    }
}

fn required_min(key: &str, min_chars: usize, errors: &mut Vec<String>) -> String {
    match raw(key) {
        None => {
            errors.push(format!("{key} is required"));
            String::new()
        }
        Some(value) => {
            if value.chars().count() < min_chars {
                errors.push(format!("{key} must be at least {min_chars} characters"));
            }
            value
        }
    }
}

fn parse<T>(key: &str, default: &str, errors: &mut Vec<String>) -> T
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    let value = with_default(key, default);
    match value.parse::<T>() {
        Ok(parsed) => parsed,
        Err(err) => {
            errors.push(format!("{key}='{value}' is invalid: {err}"));
            default.parse::<T>().expect("hardcoded default must parse")
        }
    }
}

fn parse_bool(key: &str, default: bool, errors: &mut Vec<String>) -> bool {
    match raw(key) {
        None => default,
        Some(value) => match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => true,
            "0" | "false" | "no" | "off" => false,
            _ => {
                errors.push(format!("{key}='{value}' must be a boolean (true/false)"));
                default
            }
        },
    }
}

fn one_of(key: &str, value: &str, allowed: &[&str], errors: &mut Vec<String>) {
    if !allowed.contains(&value) {
        errors.push(format!(
            "{key}='{value}' must be one of: {}",
            allowed.join(", ")
        ));
    }
}

fn require_positive<T>(key: &str, value: T, errors: &mut Vec<String>)
where
    T: PartialOrd + Default + std::fmt::Display,
{
    if value <= T::default() {
        errors.push(format!("{key} must be greater than 0 (got {value})"));
    }
}

fn http_url(key: &str, value: &str, errors: &mut Vec<String>) {
    if !(value.starts_with("http://") || value.starts_with("https://")) {
        errors.push(format!("{key}='{value}' must be an http(s) URL"));
    }
}

fn optional_url(key: &str, errors: &mut Vec<String>) -> Option<String> {
    let value = raw(key)?;
    http_url(key, &value, errors);
    Some(value)
}

impl Config {
    /// Load `.env` (if present) and validate the whole environment.
    pub fn from_env() -> ConfigResult<Self> {
        dotenvy::dotenv().ok();
        let mut errors: Vec<String> = Vec::new();

        // ---------------- application ----------------
        let app_env = with_default("APP_ENV", "development");
        one_of(
            "APP_ENV",
            &app_env,
            &["development", "staging", "production"],
            &mut errors,
        );
        let app_host = with_default("APP_HOST", "0.0.0.0");
        let app_port = parse::<u16>("APP_PORT", "8080", &mut errors);
        require_positive("APP_PORT", app_port, &mut errors);
        let app_secret = required_min("APP_SECRET", 16, &mut errors);
        let public_api_origin = with_default("PUBLIC_API_ORIGIN", "http://localhost:8080");
        http_url("PUBLIC_API_ORIGIN", &public_api_origin, &mut errors);

        let cors_origins: Vec<String> = with_default("CORS_ORIGINS", "http://localhost:5173")
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if cors_origins.is_empty() {
            errors.push("CORS_ORIGINS must contain at least one origin".to_string());
        }
        for origin in &cors_origins {
            http_url("CORS_ORIGINS", origin, &mut errors);
        }

        // ---------------- postgres ----------------
        let database_url = required("DATABASE_URL", &mut errors);
        if !database_url.is_empty() && !database_url.starts_with("postgres") {
            errors.push(format!(
                "DATABASE_URL='{database_url}' must be a postgres:// URL"
            ));
        }
        let database_max_connections = parse::<u32>("DATABASE_MAX_CONNECTIONS", "10", &mut errors);
        require_positive(
            "DATABASE_MAX_CONNECTIONS",
            database_max_connections,
            &mut errors,
        );

        // ---------------- redis ----------------
        let redis_url = required("REDIS_URL", &mut errors);
        if !redis_url.is_empty() && !redis_url.starts_with("redis") {
            errors.push(format!("REDIS_URL='{redis_url}' must be a redis:// URL"));
        }
        let redis_max_connections = parse::<u32>("REDIS_MAX_CONNECTIONS", "20", &mut errors);
        require_positive("REDIS_MAX_CONNECTIONS", redis_max_connections, &mut errors);

        // ---------------- auth ----------------
        let auth_username = required("AUTH_USERNAME", &mut errors);
        let auth_password = required_min("AUTH_PASSWORD", 8, &mut errors);
        let auth_secret = required_min("AUTH_SECRET", 16, &mut errors);
        let auth_session_ttl = parse::<i64>("AUTH_SESSION_TTL", "86400", &mut errors);
        require_positive("AUTH_SESSION_TTL", auth_session_ttl, &mut errors);
        let auth_cookie_secure = parse_bool("AUTH_COOKIE_SECURE", false, &mut errors);

        // ---------------- Keenable ----------------
        // Empty KEENABLE_API_KEY is allowed here (service-specific
        // `require_*` helpers fail fast in the services that need it).
        let keenable_api_key = raw("KEENABLE_API_KEY");
        let keenable_base_url = with_default("KEENABLE_BASE_URL", "https://api.keenable.ai");
        http_url("KEENABLE_BASE_URL", &keenable_base_url, &mut errors);
        let keenable_search_limit = parse::<u64>("KEENABLE_SEARCH_LIMIT", "500", &mut errors);
        require_positive("KEENABLE_SEARCH_LIMIT", keenable_search_limit, &mut errors);
        let keenable_fetch_limit = parse::<u64>("KEENABLE_FETCH_LIMIT", "300", &mut errors);
        require_positive("KEENABLE_FETCH_LIMIT", keenable_fetch_limit, &mut errors);
        let keenable_max_rps = parse::<u64>("KEENABLE_MAX_RPS", "8", &mut errors);
        require_positive("KEENABLE_MAX_RPS", keenable_max_rps, &mut errors);
        if keenable_max_rps > 10 {
            errors.push(format!(
                "KEENABLE_MAX_RPS={keenable_max_rps} exceeds the documented provider limit of 10 rps"
            ));
        }
        let keenable_search_mode = with_default("KEENABLE_SEARCH_MODE", "pro");
        one_of(
            "KEENABLE_SEARCH_MODE",
            &keenable_search_mode,
            &["pro", "realtime"],
            &mut errors,
        );

        // ---------------- fetch engine ----------------
        let fetch_backend = with_default("FETCH_BACKEND", "keenable");
        one_of(
            "FETCH_BACKEND",
            &fetch_backend,
            &["keenable", "direct"],
            &mut errors,
        );
        let fetch_max_chars = parse::<usize>("FETCH_MAX_CHARS", "50000", &mut errors);
        require_positive("FETCH_MAX_CHARS", fetch_max_chars, &mut errors);
        let fetch_timeout_seconds = parse::<u64>("FETCH_TIMEOUT_SECONDS", "20", &mut errors);
        require_positive("FETCH_TIMEOUT_SECONDS", fetch_timeout_seconds, &mut errors);
        let fetch_max_redirects = parse::<u8>("FETCH_MAX_REDIRECTS", "5", &mut errors);
        require_positive("FETCH_MAX_REDIRECTS", fetch_max_redirects, &mut errors);
        let fetch_max_per_component = parse::<u8>("FETCH_MAX_PER_COMPONENT", "8", &mut errors);
        require_positive(
            "FETCH_MAX_PER_COMPONENT",
            fetch_max_per_component,
            &mut errors,
        );

        // ---------------- LLM ----------------
        let llm_api_key = raw("LLM_API_KEY");
        let llm_base_url = with_default("LLM_BASE_URL", "https://api.openai.com/v1");
        http_url("LLM_BASE_URL", &llm_base_url, &mut errors);
        let llm_model = with_default("LLM_MODEL", "gpt-4.1-mini");
        let llm_fallback_model = with_default("LLM_FALLBACK_MODEL", "gpt-4.1");
        let llm_timeout = parse::<u64>("LLM_TIMEOUT", "120", &mut errors);
        require_positive("LLM_TIMEOUT", llm_timeout, &mut errors);
        let llm_max_tokens = parse::<u32>("LLM_MAX_TOKENS", "4096", &mut errors);
        require_positive("LLM_MAX_TOKENS", llm_max_tokens, &mut errors);

        // ---------------- OCR ----------------
        let ocr_provider = with_default("OCR_PROVIDER", "none");
        one_of(
            "OCR_PROVIDER",
            &ocr_provider,
            &["none", "paddle", "api"],
            &mut errors,
        );
        let ocr_api_key = raw("OCR_API_KEY");
        let ocr_base_url = optional_url("OCR_BASE_URL", &mut errors);
        if ocr_provider == "api" && ocr_api_key.is_none() {
            errors.push("OCR_API_KEY is required when OCR_PROVIDER=api".to_string());
        }

        // ---------------- storage ----------------
        let storage_provider = with_default("STORAGE_PROVIDER", "local");
        one_of(
            "STORAGE_PROVIDER",
            &storage_provider,
            &["local", "s3"],
            &mut errors,
        );
        let storage_path = with_default("STORAGE_PATH", "/data/storage");
        let s3_endpoint = optional_url("S3_ENDPOINT", &mut errors);
        let s3_bucket = raw("S3_BUCKET");
        let s3_access_key = raw("S3_ACCESS_KEY");
        let s3_secret_key = raw("S3_SECRET_KEY");
        if storage_provider == "s3" {
            for (key, value) in [
                ("S3_ENDPOINT", &s3_endpoint),
                ("S3_BUCKET", &s3_bucket),
                ("S3_ACCESS_KEY", &s3_access_key),
                ("S3_SECRET_KEY", &s3_secret_key),
            ] {
                if value.is_none() {
                    errors.push(format!("{key} is required when STORAGE_PROVIDER=s3"));
                }
            }
        }

        // ---------------- business limits ----------------
        let max_file_size = parse::<u64>("MAX_FILE_SIZE", "26214400", &mut errors);
        require_positive("MAX_FILE_SIZE", max_file_size, &mut errors);
        let max_tender_components = parse::<u32>("MAX_TENDER_COMPONENTS", "500", &mut errors);
        require_positive("MAX_TENDER_COMPONENTS", max_tender_components, &mut errors);
        let max_search_queries = parse::<u32>("MAX_SEARCH_QUERIES", "12", &mut errors);
        require_positive("MAX_SEARCH_QUERIES", max_search_queries, &mut errors);
        let max_fetch_requests = parse::<u32>("MAX_FETCH_REQUESTS", "300", &mut errors);
        require_positive("MAX_FETCH_REQUESTS", max_fetch_requests, &mut errors);
        let max_llm_tokens_per_tender =
            parse::<u64>("MAX_LLM_TOKENS_PER_TENDER", "2000000", &mut errors);
        require_positive(
            "MAX_LLM_TOKENS_PER_TENDER",
            max_llm_tokens_per_tender,
            &mut errors,
        );
        let max_tender_duration = parse::<u64>("MAX_TENDER_DURATION", "7200", &mut errors);
        require_positive("MAX_TENDER_DURATION", max_tender_duration, &mut errors);

        // ---------------- concurrency ----------------
        let worker_concurrency = parse::<usize>("WORKER_CONCURRENCY", "4", &mut errors);
        require_positive("WORKER_CONCURRENCY", worker_concurrency, &mut errors);
        let search_concurrency = parse::<usize>("SEARCH_CONCURRENCY", "4", &mut errors);
        require_positive("SEARCH_CONCURRENCY", search_concurrency, &mut errors);
        let ai_concurrency = parse::<usize>("AI_CONCURRENCY", "2", &mut errors);
        require_positive("AI_CONCURRENCY", ai_concurrency, &mut errors);
        let document_concurrency = parse::<usize>("DOCUMENT_CONCURRENCY", "2", &mut errors);
        require_positive("DOCUMENT_CONCURRENCY", document_concurrency, &mut errors);

        // ---------------- observability ----------------
        let log_level = with_default("LOG_LEVEL", "info");
        one_of(
            "LOG_LEVEL",
            &log_level,
            &["error", "warn", "info", "debug", "trace"],
            &mut errors,
        );
        let otel_exporter_otlp_endpoint = raw("OTEL_EXPORTER_OTLP_ENDPOINT");
        let sentry_dsn = raw("SENTRY_DSN");

        // ---------------- production hardening ----------------
        if app_env == "production" {
            if !auth_cookie_secure {
                errors.push("AUTH_COOKIE_SECURE must be true when APP_ENV=production".to_string());
            }
            if app_secret == "change-me-internal-secret" {
                errors.push("APP_SECRET must be changed in production".to_string());
            }
            if auth_password == "change-me-strong-password" {
                errors.push("AUTH_PASSWORD must be changed in production".to_string());
            }
            if auth_secret == "change-me-auth-secret" {
                errors.push("AUTH_SECRET must be changed in production".to_string());
            }
            if public_api_origin.starts_with("http://localhost") {
                errors.push(
                    "PUBLIC_API_ORIGIN must not be localhost when APP_ENV=production".to_string(),
                );
            }
        }

        if !errors.is_empty() {
            return Err(ConfigError(errors.join("; ")));
        }

        Ok(Config {
            app_env,
            app_host,
            app_port,
            app_secret,
            public_api_origin,
            cors_origins,
            database_url,
            database_max_connections,
            redis_url,
            redis_max_connections,
            auth_username,
            auth_password,
            auth_secret,
            auth_session_ttl,
            auth_cookie_secure,
            keenable_api_key,
            keenable_base_url,
            keenable_search_limit,
            keenable_fetch_limit,
            keenable_max_rps,
            keenable_search_mode,
            fetch_backend,
            fetch_max_chars,
            fetch_timeout_seconds,
            fetch_max_redirects,
            fetch_max_per_component,
            llm_api_key,
            llm_base_url,
            llm_model,
            llm_fallback_model,
            llm_timeout,
            llm_max_tokens,
            ocr_provider,
            ocr_api_key,
            ocr_base_url,
            storage_provider,
            storage_path,
            s3_endpoint,
            s3_bucket,
            s3_access_key,
            s3_secret_key,
            max_file_size,
            max_tender_components,
            max_search_queries,
            max_fetch_requests,
            max_llm_tokens_per_tender,
            max_tender_duration,
            worker_concurrency,
            search_concurrency,
            ai_concurrency,
            document_concurrency,
            log_level,
            otel_exporter_otlp_endpoint,
            sentry_dsn,
        })
    }
}

impl Config {
    /// Keenable key is mandatory for the search service; fail fast at startup.
    pub fn require_keenable_api_key(&self) -> ConfigResult<&str> {
        self.keenable_api_key
            .as_deref()
            .filter(|key| !key.is_empty())
            .ok_or_else(|| ConfigError("KEENABLE_API_KEY is required for this service".into()))
    }

    /// LLM key is mandatory for AI extraction workers.
    pub fn require_llm_api_key(&self) -> ConfigResult<&str> {
        self.llm_api_key
            .as_deref()
            .filter(|key| !key.is_empty())
            .ok_or_else(|| ConfigError("LLM_API_KEY is required for this service".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard};

    /// Tests that call `Config::from_env` read the whole process environment
    /// and must not run concurrently with each other. CI provides a clean
    /// environment (no `.env` file, none of these variables set).
    fn env_guard() -> MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Sets variables for the duration of a test and removes them even when
    /// an assertion fails.
    struct EnvScope(&'static [(&'static str, &'static str)]);

    impl Drop for EnvScope {
        fn drop(&mut self) {
            for (key, _) in self.0 {
                env::remove_var(key);
            }
        }
    }

    #[test]
    fn required_min_reports_missing_and_short_values() {
        let key = "TENDER_TEST_REQUIRED_MIN";
        let _guard = env_guard();
        env::remove_var(key);

        let mut errors = Vec::new();
        assert_eq!(required_min(key, 8, &mut errors), "");
        assert_eq!(errors, [format!("{key} is required")]);

        env::set_var(key, "short");
        let mut errors = Vec::new();
        assert_eq!(required_min(key, 8, &mut errors), "short");
        assert_eq!(errors, [format!("{key} must be at least 8 characters")]);
        env::remove_var(key);
    }

    #[test]
    fn parse_invalid_value_reports_error_and_falls_back_to_default() {
        let key = "TENDER_TEST_BAD_U16";
        let _guard = env_guard();
        env::set_var(key, "banana");

        let mut errors = Vec::new();
        assert_eq!(parse::<u16>(key, "8080", &mut errors), 8080);
        assert_eq!(errors.len(), 1);
        assert!(
            errors[0].contains(key),
            "error must name the variable: {errors:?}"
        );
        env::remove_var(key);
    }

    #[test]
    fn parse_bool_accepts_spellings_and_rejects_garbage() {
        let key = "TENDER_TEST_BOOL";
        let _guard = env_guard();
        let mut errors = Vec::new();

        env::set_var(key, "true");
        assert!(parse_bool(key, false, &mut errors));
        env::set_var(key, "0");
        assert!(!parse_bool(key, true, &mut errors));
        env::set_var(key, "banana");
        assert!(!parse_bool(key, false, &mut errors));

        assert_eq!(errors.len(), 1, "only garbage must be reported: {errors:?}");
        env::remove_var(key);
    }

    #[test]
    fn production_rejects_insecure_and_default_secrets() {
        let _guard = env_guard();
        let vars: &'static [(&'static str, &'static str)] = &[
            ("APP_ENV", "production"),
            ("APP_SECRET", "change-me-internal-secret"),
            ("DATABASE_URL", "postgres://user:pass@localhost:5432/db"),
            ("REDIS_URL", "redis://localhost:6379"),
            ("AUTH_USERNAME", "admin"),
            ("AUTH_PASSWORD", "change-me-strong-password"),
            ("AUTH_SECRET", "change-me-auth-secret"),
            ("AUTH_COOKIE_SECURE", "false"),
            ("PUBLIC_API_ORIGIN", "http://localhost:8080"),
        ];
        let _scope = EnvScope(vars);
        for (key, value) in vars {
            env::set_var(key, value);
        }

        let err = Config::from_env().expect_err("production defaults must fail validation");
        assert!(err.0.contains("AUTH_COOKIE_SECURE must be true"), "{err}");
        assert!(err.0.contains("APP_SECRET must be changed"), "{err}");
        assert!(err.0.contains("AUTH_PASSWORD must be changed"), "{err}");
        assert!(err.0.contains("AUTH_SECRET must be changed"), "{err}");
        assert!(
            err.0.contains("PUBLIC_API_ORIGIN must not be localhost"),
            "{err}"
        );
    }

    #[test]
    fn valid_environment_parses() {
        let _guard = env_guard();
        let vars: &'static [(&'static str, &'static str)] = &[
            ("APP_ENV", "development"),
            ("APP_SECRET", "0123456789abcdef"),
            ("DATABASE_URL", "postgres://user:pass@localhost:5432/db"),
            ("REDIS_URL", "redis://localhost:6379"),
            ("AUTH_USERNAME", "operator"),
            ("AUTH_PASSWORD", "sufficiently-long-password"),
            ("AUTH_SECRET", "0123456789abcdef"),
        ];
        let _scope = EnvScope(vars);
        for (key, value) in vars {
            env::set_var(key, value);
        }

        let config = Config::from_env().expect("valid environment must parse");
        assert_eq!(config.app_port, 8080);
        assert_eq!(config.fetch_backend, "keenable");
        assert_eq!(config.keenable_search_mode, "pro");
        assert!(
            !config.auth_cookie_secure,
            "default is false outside production"
        );
    }
}
