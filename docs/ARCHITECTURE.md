# Architecture

## Components

| Component | Language | Responsibility | Entry point |
| --- | --- | --- | --- |
| API Gateway | Rust (Axum) | Auth, tenders, documents, requirements, results, reports, WebSocket progress, migrations | `services/rust/crates/api` |
| Orchestrator | Rust | **Sole writer of the tender FSM**; drives every stage over Redis Streams; matching/ranking via gRPC; run watchdog | `services/rust/crates/orchestrator` |
| Search Engine | Rust | Keenable client (rate limit + Retry-After), deterministic query generation, relevance prefilter, SSRF-guarded fetch | `services/rust/crates/search` |
| Matching Engine | Rust | Pure technical matcher (per-requirement checks) and explainable ranking | `services/rust/crates/matching` |
| Document Worker | Python 3.12 | PDF/DOCX/XLSX/TXT/RTF parsing into `document_pages`, optional OCR | `services/python/document_worker` |
| AI Worker | Python 3.12 | Requirement extraction and product/spec extraction with strict provenance validation | `services/python/ai_worker` |
| Report Worker | Python 3.12 | XLSX / PDF / DOCX reports with score breakdowns and requirement checks | `services/python/report_worker` |
| Frontend | React + TS | Operator console (tenders, positions, results, reports, activity, usage) | `services/frontend` |

`proto/` is the single source of truth for every inter-service contract.
Protobuf code is generated at build time (cargo build scripts, Dockerfiles,
`scripts/gen-proto-python.sh`) and never committed.

## Transport

* **gRPC** — synchronous calls: the orchestrator calls `MatchingService.MatchComponent`
  and `MatchingService.RankOffers`; `SearchService` exposes search/fetch for ad-hoc
  calls and health aggregation.
* **Redis Streams** — asynchronous jobs and events (at-least-once delivery):

```
API ──jobs:documents──▶ document-worker ──events:documents──▶ orchestrator
API ──jobs:control────▶ orchestrator ──jobs:ai.requirements──▶ ai-worker
                          │  ──jobs:search──────────────────▶ search ──events:search──▶ orchestrator
                          │  ──jobs:ai.products─────────────▶ ai-worker ──events:ai──▶ orchestrator
                          └─ ──jobs:reports────────────────▶ report-worker ──events:reports──▶ orchestrator
```

Progress is published as serialized `ProgressEvent` protobuf on the Pub/Sub
channel `events:progress`; the API decodes it and fans JSON out over WebSocket.

## Tender state machine

```
CREATED → UPLOADING → DOCUMENT_PROCESSING → REQUIREMENT_EXTRACTION
        → REQUIREMENTS_REVIEW → SEARCH_QUEUED → SEARCHING → FETCHING
        → PRODUCT_EXTRACTION → MATCHING → RANKING → REPORT_GENERATION
        → COMPLETED | COMPLETED_WITH_ERRORS
```

Any non-terminal state may move to `FAILED` or `CANCELLED`. Terminal states are
immutable. Transitions are validated in `services/rust/crates/orchestrator/src/fsm.rs`
(pure, unit-tested) and applied under `SELECT ... FOR UPDATE`; the API never
writes `tenders.status`.

Tender-level stages are aggregate milestones derived from the run's task
statuses (`aggregate_target` in `orchestrator/src/state.rs`): the tender only
advances when *every* non-cancelled position has reached that stage.

Task lifecycle: `QUEUED → SEARCHING → FETCHING → EXTRACTING → MATCHING → DONE`
(plus `FAILED`/`CANCELLED`).

## Delivery guarantees

* Streams use consumer groups (`doc-workers`, `ai-workers`, `search-workers`,
  `report-workers`, `orchestrator`) created from `0`, so nothing published
  before a worker's first boot is skipped.
* Workers deduplicate by logical task id with Redis `SET NX` (TTL 6 h) and by
  PostgreSQL unique constraints (`search_tasks(run_id, component_id)`,
  `search_results(task_id, url)`, `search_queries(task_id, query)`,
  `sources(original_url, content_hash)`, `products(content_hash)`).
* Infrastructure failures leave the message in the PEL and release the dedup
  key, so `XAUTOCLAIM` (after 10 minutes idle) retries it. A message failing
  five times is dropped **with an ERROR log**.
* `MAX_TENDER_DURATION` is enforced by the orchestrator watchdog (60 s tick).

## Honest limitations (v1)

* Object storage: only `STORAGE_PROVIDER=local` is implemented; `s3` is refused
  at startup instead of silently losing files.
* OTLP export: `LOG_LEVEL` and `RUST_LOG` are fully wired; setting
  `OTEL_EXPORTER_OTLP_ENDPOINT` logs a warning that no exporter is configured yet.
* OCR requires an external HTTP OCR service (`OCR_PROVIDER=api`). Without it,
  pages lacking a text layer are stored as empty `image` pages.
* Single operator account (no registration, no multi-tenancy).
