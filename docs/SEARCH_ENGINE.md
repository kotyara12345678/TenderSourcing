# Search engine (Keenable)

Provider: <https://keenable.ai>, API docs <https://docs.keenable.ai>.
Base URL: `KEENABLE_BASE_URL`, auth header `X-API-Key: $KEENABLE_API_KEY`.

## Calls

| Purpose | Call | Notes |
| --- | --- | --- |
| Web search | `POST /v1/search` | `{query, mode, site?, published_after?, published_before?, snippet_max_length?, max_results?}` |
| Page fetch | `GET /v1/fetch?url=…&max_chars=…&live=…&prompt=…` | `max_chars` default 50000, `prompt ≤ 2000` |
| Health probe | `GET /v1/fetch/public?url=https://example.com` | free, unauthenticated, needs `X-Keenable-Title` |

`mode` is `pro` (default) or `realtime`. `snippet_max_length ∈ [180, 10000]`,
`max_results ∈ [1, 50]`; values are validated client-side before the call.

## Rate limiting

* A shared token-slot limiter spaces requests evenly at `KEENABLE_MAX_RPS`
  (default 8; the provider's organization limit is 10 rps — keep headroom).
* `429` responses push the next allowed slot out by `Retry-After` (seconds,
  capped at 60); after three attempts the call fails with `rate_limited`.
* `5xx` responses are retried up to three attempts with linear backoff.
* Other `4xx` fail fast with an upstream error; provider payloads are truncated
  to 500 characters and never include the API key.

## Budgets

* `KEENABLE_SEARCH_LIMIT` — monthly search quota (used to cap generated queries).
* `MAX_SEARCH_QUERIES` — queries per tender position (default 12).
* `MAX_FETCH_REQUESTS` / `FETCH_MAX_PER_COMPONENT` — page fetch budget.
* `KEENABLE_FETCH_LIMIT` — monthly fetch quota.

Every paid call is recorded in `api_usage` with the provider's `sku`, credits
and (when reported) price — accounting is never invented.

## Query generation

`services/rust/crates/search/src/queries.rs` builds a deterministic list per
position, no randomness:

1. `exact` — the position name;
2. `alt` — `search_hints.alternate_names` / `english_name`;
3. `manufacturer` — `"{manufacturer} {name}"`;
4. `model` — `"{manufacturer} {model}"`;
5. `specs` — requirements of type `STANDARD`, `MATERIAL`, `MODEL`, `DIMENSION`
   or numeric values (`"{requirement} {value} {unit}"`);
6. `custom` — `search_hints.suggested_queries`;
7. `synonym` — full-name RU→EN dictionary translation (only when every
   significant token is covered — partial translations are skipped);
8. `multilingual` — Cyrillic names get `"… specification datasheet"`, Latin
   names get `"… характеристики"`.

Duplicates are removed after whitespace/lowercase normalization and the list is
capped by the remaining budget.

## Prefilter and fetch

* Relevance score (0..1) = `0.7 × term coverage (title+snippet) + 0.3 × term
  coverage in title + 0.2` when the whole normalized query appears verbatim
  (clamped to 1).
* Pages below `0.05` are never fetched (no credit spend on irrelevant pages).
* The top `FETCH_MAX_PER_COMPONENT` pages per position are fetched; a single
  unreachable page never fails the position.

## Fetch backends

* `FETCH_BACKEND=keenable` (default) — provider fetch, markdown content.
* `FETCH_BACKEND=direct` — built-in crawler: redirects validated hop by hop,
  blocked hostnames (`localhost`, `*.local`, `*.internal`, metadata endpoints),
  private/loopback/link-local/CGNAT/documentation/multicast addresses rejected,
  `FETCH_MAX_REDIRECTS` budget, HTML stripped to text.
  Known limitation: the address is resolved twice (check → connect), so a DNS
  rebinding window exists; see `docs/SECURITY.md`.

## Spent credits

`search_runs.search_credits` / `fetch_credits` accumulate provider-reported
usage; `api_usage` keeps the per-call rows. Both are visible in the UI
(`/usage`).
