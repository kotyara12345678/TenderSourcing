# Troubleshooting

## The API exits at startup

The configuration validator reports **every** problem at once. Typical causes:

* `APP_SECRET` / `AUTH_SECRET` shorter than 32 characters or containing `change-me`
  (production);
* `AUTH_COOKIE_SECURE=false` in production;
* `PUBLIC_API_ORIGIN` pointing at `localhost` in production;
* `KEENABLE_MAX_RPS > 10`;
* missing `DATABASE_URL` / `REDIS_URL`.

Fix the listed variables and restart.

## `STORAGE_PROVIDER='s3' is not implemented`

v1 supports only local storage. Set `STORAGE_PROVIDER=local` and mount a shared
volume at `STORAGE_PATH` for the API, document worker and report worker.

## Upload rejected

* `unsupported file type` — allowed: pdf, docx, xlsx, txt, rtf, odt;
* `file content does not match the extension` — magic bytes differ (e.g. a DOCX
  renamed to PDF);
* `file exceeds the maximum upload size` — raise `MAX_FILE_SIZE` and nginx
  `client_max_body_size` together.

## Tender stuck in `DOCUMENT_PROCESSING`

1. `docker compose -f deploy/docker-compose.yml logs document-worker`
2. Check `documents` rows: `status`, `error_message`.
3. `XLEN jobs:documents` and `XPENDING jobs:documents doc-workers` — pending
   entries mean a worker died; they are re-claimed after 10 minutes idle
   (`XAUTOCLAIM`).
4. Corrupt files are marked `FAILED` with the parser error; re-upload a fixed
   version.

## Tender stuck in `REQUIREMENT_EXTRACTION`

Usually a missing/invalid `LLM_API_KEY` or an unreachable `LLM_BASE_URL`.
`documents.ai_status` shows `RUNNING`/`FAILED`; the AI worker log contains the
provider error. Positions extracted so far stay in the database.

## `search can start from REQUIREMENTS_REVIEW...`

The start button only works in that state. Waiting for the pipeline? Check the
orchestrator log for `tender transition` lines and
`SELECT status FROM documents WHERE tender_id = …` — extraction is complete when
every document is `EXTRACTED`/`FAILED` and `ai_status` is `DONE`/`FAILED`.

## No search results / empty `search_results`

* `SELECT kind, results_count, error FROM search_queries WHERE task_id = …` —
  every generated query and its provider outcome.
* `429` errors: the limiter honors `Retry-After`; reduce `KEENABLE_MAX_RPS`.
* Budget exhaustion (`search query budget for this tender is exhausted`): the
  monthly `KEENABLE_SEARCH_LIMIT` is used up.

## Pages fetched but no products

`products` are extracted by the AI worker with provenance checks. If every spec
in a page fails the verbatim-snippet check, the product is dropped (by design).
Inspect `ai-worker` logs for `dropping spec ... not verbatim`.

## Report failed

* `DejaVu fonts are required for Cyrillic PDF output` — the worker image must
  install `fonts-dejavu-core` (already in `deploy/docker/Dockerfile.worker`); a
  custom image without it cannot render Russian text.
* `report is not ready yet` — the file row exists but generation failed; check
  `reports.error_message` and `report-worker` logs.

## WebSocket does not update

The socket authenticates with the same cookie as the API. Check:

1. `docker compose logs frontend` — nginx must proxy `/api/v1/ws/` with
   `Upgrade`/`Connection` headers (shipped config does).
2. Browser devtools: the WS request returns 404 when the tender id is wrong or
   the tender was deleted; 401 when the session expired.

## Worker health

`curl http://localhost:8090/healthz` inside a worker container; `/readyz`
additionally checks PostgreSQL and Redis (the search service also probes
Keenable via the free public fetch endpoint).

## Reset for a clean test

```bash
docker compose -f deploy/docker-compose.yml down -v   # removes pg/redis/storage volumes
docker compose -f deploy/docker-compose.yml up -d --build
```

This destroys all data — only for development environments.
