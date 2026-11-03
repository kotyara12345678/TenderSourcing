# Security

## Authentication and sessions

* Single operator account, created from ENV on first start; password hashed with
  **argon2id** (default parameters, random salt).
* `password_fingerprint = HMAC-SHA256(APP_SECRET, AUTH_PASSWORD)` detects ENV
  rotation without storing the password; rotation re-hashes and revokes every
  session.
* Sessions are server-side rows. The cookie `ts_session` is `HttpOnly`,
  `SameSite=Strict`, `Path=/`, `Secure` when `AUTH_COOKIE_SECURE=true` (forced in
  production). Only `SHA-256(token)` is stored, so a database leak does not leak
  usable cookies. Tokens carry 244 bits of entropy from two UUIDv4 values.
* Every mutating request requires `X-CSRF-Token` matching the session's token
  (constant-time comparison) in addition to `SameSite`.
* Sessions expire after `AUTH_SESSION_TTL`; `last_seen_at` slides at most once
  per five minutes to limit write amplification.

## Authorization

v1 has a single operator: authenticated == authorized. Tender deletion is
soft (`deleted_at`), and every mutating endpoint validates the tender state
before touching data. Requests for another user's tender are impossible because
there is exactly one user; when multi-tenancy lands, every query must be scoped
by `user_id`.

## Input validation

* Uploads: extension allowlist, magic-byte checks (`%PDF`, `PK\x03\x04`),
  `MAX_FILE_SIZE`, `sha256` deduplication.
* Storage keys: rejected if absolute or containing `..`; files live under
  `STORAGE_PATH` only.
* Numeric/enum fields validated against the DB CHECK constraints before writing
  (`data_type`, tolerance range, quantity > 0, range_min ≤ range_max).
* Pagination clamped (`page_size ≤ 100`), list filters trimmed.

## SSRF (direct fetch backend)

Before each hop: scheme must be `http`/`https`, no credentials in the URL, host
must not be `localhost`/`*.local`/`*.internal`/metadata endpoints, and **every**
resolved address must be public (private, loopback, link-local, CGNAT,
documentation, multicast, unspecified and IPv4-mapped IPv6 addresses are
rejected). Redirects are never followed automatically; the budget is
`FETCH_MAX_REDIRECTS`.

**Known limitation:** the address is resolved for the check and again by the
HTTP connector, leaving a DNS-rebinding window. Mitigations: restrict outbound
traffic at the network level, keep the default `FETCH_BACKEND=keenable`, and
treat the direct backend as a fallback for controlled environments.

## Secrets

* Secrets exist only in `.env` (git-ignored) or the orchestrator's secret store;
  `.env.example` contains no real values.
* The Keenable key travels in `X-API-Key` only; provider error bodies are
  truncated to 500 characters and never include the key.
* LLM calls use `Authorization: Bearer` server-side; the browser never sees any
  provider key.
* Logs never contain request bodies with secrets; the API's internal errors are
  reduced to `internal server error` for clients.

## Network exposure

Only the frontend port is published by Compose. PostgreSQL, Redis, gRPC
(50051/50052) and worker health ports are internal. TLS must terminate in front
of nginx.

## Content trust

Pages fetched from the web are untrusted input:

* the AI worker requires verbatim `source_snippet` quotes to accept a value;
* the matcher never treats missing data as a match (`INSUFFICIENT` verdict);
* prices/lead times remain `null` unless the page states them;
* the UI renders links with `rel="noreferrer noopener"` and never injects HTML
  (`dangerouslySetInnerHTML` is not used anywhere).

## What is intentionally absent

* No user registration, password reset by email, or multi-factor auth (single
  operator, ENV-managed credentials).
* No public API tokens: the WebSocket and REST API are cookie-only.
* No S3 provider implementation — startup fails instead of silently storing
  documents elsewhere.
