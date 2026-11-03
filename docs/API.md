# HTTP API

Base path: `/api/v1`. Authentication: HttpOnly session cookie `ts_session`
(`SameSite=Strict`); every state-changing request additionally requires the
header `X-CSRF-Token` with the value returned by `/auth/login` and `/auth/me`.

Errors use a uniform envelope:

```json
{ "error": { "code": "conflict", "message": "search can start from REQUIREMENTS_REVIEW, the tender is in SEARCHING" } }
```

Internal failures (database, Redis, storage) are logged in full server-side and
returned as `{"error":{"code":"internal","message":"internal server error"}}`.

## Auth

| Method | Path | Body | Notes |
| --- | --- | --- | --- |
| POST | `/auth/login` | `{username,password}` | Sets the session cookie; returns `{user:{id,username,display_name,csrf_token,expires_at}}` |
| POST | `/auth/logout` | — | Revokes the session, clears the cookie (CSRF required) |
| GET | `/auth/me` | — | Current user + CSRF token |

## Tenders

| Method | Path | Notes |
| --- | --- | --- |
| GET | `/tenders?page&page_size&status&q` | Paginated list with position counts |
| POST | `/tenders` | `{name, description}` |
| GET | `/tenders/{id}` | Detail + document/report counts + latest run |
| PATCH | `/tenders/{id}` | `{name?, description?}` (finished tenders are immutable) |
| DELETE | `/tenders/{id}` | Soft delete; requires cancellation for active runs |
| POST | `/tenders/{id}/cancel` | Queues a `cancel` control message → orchestrator |
| POST | `/tenders/{id}/start-search` | Creates a run + tasks, queues `start_run`; allowed from `REQUIREMENTS_REVIEW` |

## Documents

| Method | Path | Notes |
| --- | --- | --- |
| GET | `/tenders/{id}/documents` | List with parse and AI status |
| POST | `/tenders/{id}/documents` | `multipart/form-data`, field `file`; deduplicated by SHA-256 |
| GET | `/documents/{id}` | Detail + first three page previews |
| DELETE | `/documents/{id}` | Soft delete (refused while queued/processing) |

Allowed uploads: `pdf`, `docx`, `xlsx`, `txt`, `rtf`, `odt`; magic bytes are
checked for PDF (`%PDF`) and ZIP containers (`PK\x03\x04`); size limit
`MAX_FILE_SIZE` (default 25 MiB).

## AI intake (free-form request)

The operator can skip the document entirely: a request written in plain
language is turned into positions, quantities and the characteristics that were
actually stated.

| Method | Path | Notes |
| --- | --- | --- |
| POST | `/assistant/parse` | `{text}` (8…4000 chars) → `202 {id}`; queues `ParseIntakeJob` on `jobs:ai.intake` |
| GET | `/assistant/{id}` | `{id, status, progress, steps[], result, error_message, tender_id}` |
| POST | `/assistant/{id}/apply` | `{name, description, positions[]}` → `201 {tender_id, positions, requirements}` |

The row is created before the job is queued, so the client polls one id from the
first millisecond: `QUEUED → RUNNING → COMPLETED | FAILED`, then `APPLIED` once
a tender exists. `steps` is the agent trace (four real stages: read, structure,
validate, save) and `progress` is `0..1`.

The model output is a proposal only: `apply` builds the tender from the body the
operator confirmed, inside one transaction. Requirements whose value is not
present in the request text are dropped before the result is stored — numbers
are compared exactly as written, the same anti-hallucination rule the document
extractor applies with verbatim snippets.

## Positions and requirements

| Method | Path | Notes |
| --- | --- | --- |
| GET | `/tenders/{id}/components` | Positions with requirement counters |
| POST | `/tenders/{id}/components` | `{name, quantity, unit, category}` (manual position) |
| PATCH | `/components/{id}` | Same body |
| DELETE | `/components/{id}` | Cascades requirements |
| GET | `/components/{id}/requirements` | List |
| POST | `/components/{id}/requirements` | `{name, value_text, value_num, unit, data_type, mandatory, range_min, range_max, tolerance, allowed_values}` |
| PATCH | `/requirements/{id}` | Same body |
| DELETE | `/requirements/{id}` | — |

Editing is only allowed while the tender is `CREATED`, `UPLOADING`,
`DOCUMENT_PROCESSING`, `REQUIREMENT_EXTRACTION` or `REQUIREMENTS_REVIEW`.

`data_type` must be one of `STRING, NUMBER, RANGE, BOOLEAN, ENUM, STANDARD,
MATERIAL, MODEL, DIMENSION`.

## Results and reports

| Method | Path | Notes |
| --- | --- | --- |
| GET | `/tenders/{id}/results` | Positions with all ranked matches |
| GET | `/components/{id}/matches` | Matches + per-requirement checks |
| GET | `/tenders/{id}/runs` | Search runs with counters |
| GET | `/tenders/{id}/reports` | Generated files |
| GET | `/reports/{id}/download` | File stream (`attachment`), only `READY` reports |

## Activity and usage

| Method | Path | Notes |
| --- | --- | --- |
| GET | `/tenders/{id}/activity?limit` | Tender activity feed (default 50, max 200) |
| GET | `/activity?limit` | Global feed |
| GET | `/usage/summary` | Totals, per service, per tender, recent calls |

## WebSocket

`GET /api/v1/ws/tenders/{id}` — same cookie auth. The first frame is a snapshot
(`{type:"snapshot", status, components, matched}`), then every progress event
for that tender (`{type:"progress", stage, progress, message, level, ...}`).

## Health

`GET /healthz` (liveness) and `GET /readyz` (PostgreSQL + Redis) live at the
root, outside `/api/v1`.
