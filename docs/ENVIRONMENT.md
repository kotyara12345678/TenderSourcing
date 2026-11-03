# Environment variables

`.env.example` documents every variable; this page explains intent and the
validation that runs at startup (`services/rust/crates/common/src/config.rs`).

## Application

| Variable | Default | Notes |
| --- | --- | --- |
| `APP_ENV` | `development` | `production` enables hardening checks |
| `APP_HOST` / `APP_PORT` | `0.0.0.0` / `8080` | API bind address |
| `APP_SECRET` | — | ≥32 chars; used for password fingerprints |
| `PUBLIC_API_ORIGIN` | — | public origin; must not be `localhost` in production |
| `CORS_ORIGINS` | empty | comma-separated; empty = same-origin only |

## Data stores

| Variable | Default | Notes |
| --- | --- | --- |
| `DATABASE_URL` | — | `postgres://user:pass@postgres:5432/tendersourcing` |
| `DATABASE_MAX_CONNECTIONS` | `10` | pool size per Rust service; Python uses 5 |
| `REDIS_URL` | — | `redis://redis:6379/0` |
| `REDIS_MAX_CONNECTIONS` | `10` | reserved for future pooling |

## Auth

| Variable | Default | Notes |
| --- | --- | --- |
| `AUTH_USERNAME` | — | operator login |
| `AUTH_PASSWORD` | — | operator password (argon2id stored; never logged) |
| `AUTH_SECRET` | — | ≥32 chars, independent from `APP_SECRET` |
| `AUTH_SESSION_TTL` | `86400` | session lifetime in seconds |
| `AUTH_COOKIE_SECURE` | `true` in production | must be `true` behind TLS |

## Keenable

| Variable | Default | Notes |
| --- | --- | --- |
| `KEENABLE_API_KEY` | — | required by the search service (fails fast) |
| `KEENABLE_BASE_URL` | `https://api.keenable.ai` | |
| `KEENABLE_SEARCH_LIMIT` | `10000` | monthly search quota |
| `KEENABLE_FETCH_LIMIT` | `10000` | monthly fetch quota |
| `KEENABLE_MAX_RPS` | `8` | validated ≤ 10 (provider org limit) |
| `KEENABLE_SEARCH_MODE` | `pro` | `pro` or `realtime` |

## Fetch engine

| Variable | Default | Notes |
| --- | --- | --- |
| `FETCH_BACKEND` | `keenable` | `keenable` or `direct` |
| `FETCH_MAX_CHARS` | `50000` | stored content cap per page |
| `FETCH_TIMEOUT_SECONDS` | `30` | per request |
| `FETCH_MAX_REDIRECTS` | `5` | direct backend only |
| `FETCH_MAX_PER_COMPONENT` | `8` | pages fetched per position |

## LLM / OCR

| Variable | Default | Notes |
| --- | --- | --- |
| `LLM_API_KEY` | — | required by the AI worker |
| `LLM_BASE_URL` | `https://api.openai.com/v1` | any OpenAI-compatible gateway |
| `LLM_MODEL` | `gpt-4o-mini` | extraction model |
| `LLM_FALLBACK_MODEL` | empty | used when a job sets `escalate` |
| `LLM_TIMEOUT` | `120` | seconds |
| `LLM_MAX_TOKENS` | `8000` | per completion |
| `LLM_TEMPERATURE` | `0` | keep 0 for reproducibility |
| `OCR_PROVIDER` | `none` | `none` or `api` |
| `OCR_API_KEY` / `OCR_BASE_URL` | empty | required when `OCR_PROVIDER=api` |

## Storage

| Variable | Default | Notes |
| --- | --- | --- |
| `STORAGE_PROVIDER` | `local` | `s3` is refused in v1 |
| `STORAGE_PATH` | `/data/storage` | shared volume mount |
| `S3_*` | empty | reserved; unused in v1 |

## Limits and concurrency

| Variable | Default | Notes |
| --- | --- | --- |
| `MAX_FILE_SIZE` | `26214400` | 25 MiB |
| `MAX_TENDER_COMPONENTS` | `500` | positions per tender |
| `MAX_SEARCH_QUERIES` | `12` | per position |
| `MAX_FETCH_REQUESTS` | `300` | per run |
| `MAX_LLM_TOKENS_PER_TENDER` | `2000000` | hard budget |
| `MAX_TENDER_DURATION` | `7200` | watchdog seconds |
| `WORKER_CONCURRENCY` | `4` | Python workers |
| `SEARCH_CONCURRENCY` | `4` | Rust search consumers |
| `AI_CONCURRENCY` | `2` | AI worker pairs |
| `DOCUMENT_CONCURRENCY` | `2` | document consumers |

## Service mesh and observability

| Variable | Default | Notes |
| --- | --- | --- |
| `MATCHING_GRPC_URL` | `http://matching:50051` | orchestrator → matching |
| `SEARCH_GRPC_URL` | `http://search:50052` | orchestrator → search |
| `HTTP_PORT` | `8090` | worker health endpoint |
| `LOG_LEVEL` | `info` | `RUST_LOG` overrides per service |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | empty | warns: exporter not wired yet |
| `SENTRY_DSN` | empty | reserved |

## Frontend

| Variable | Default | Notes |
| --- | --- | --- |
| `VITE_API_BASE_URL` | `/api/v1` | build-time; nginx proxies `/api/` |
| `FRONTEND_PORT` | `8088` | published Compose port |

## Production validation

The API refuses to start in production when:

* `APP_SECRET` / `AUTH_SECRET` are shorter than 32 characters or contain
  `change-me`;
* `AUTH_COOKIE_SECURE=false`;
* `PUBLIC_API_ORIGIN` points at `localhost`;
* `KEENABLE_MAX_RPS > 10`;
* any required variable is missing.

All problems are reported together in a single message.
