# Database

PostgreSQL 16. Migrations live in `migrations/` and are executed **only** by the
API gateway at startup (`sqlx::migrate!`). Workers never migrate.

| Migration | Content |
| --- | --- |
| `20261102000001_core_auth_tenders.sql` | `users`, `sessions`, `tenders`, `documents`, `document_pages`, `activity_logs`, `system_settings`, `set_updated_at()` trigger function |
| `20261102000002_components_requirements.sql` | `components`, `requirements` |
| `20261102000003_search_matching_reports.sql` | `search_runs`, `search_tasks`, `search_queries`, `sources`, `search_results`, `suppliers`, `products`, `product_specs`, `matches`, `match_details`, `reports`, `api_usage` |
| `20261102000004_document_ai_status.sql` | `documents.ai_status` (`PENDING/RUNNING/DONE/FAILED`) + index |

## Key tables

* `tenders.status` — 16-state FSM (see `docs/ARCHITECTURE.md`), CHECK-constrained.
* `documents` — one physical upload; `sha256` unique per tender; `ai_status`
  tracks requirement extraction per document.
* `document_pages` — page text, tables and positioned blocks (JSONB), unique on
  `(document_id, page_number)`.
* `components` — tender positions; `search_hints` JSONB holds
  `alternate_names`, `english_name`, `manufacturer`, `model`, `suggested_queries`.
* `requirements` — one checkable constraint; `tolerance` is a fraction (0.05 = 5%).
* `search_runs` / `search_tasks` — a run and its per-position tasks
  (`UNIQUE(run_id, component_id)`).
* `search_queries` — generated queries (`kind ∈ exact, alt, manufacturer, model,
  specs, synonym, multilingual, custom`), unique per task.
* `sources` — fetched pages with full provenance: final and original URL,
  provider, content hash, HTTP status, content, snippet.
* `search_results` — deduplicated per task by URL, with `relevance_score`.
* `products` / `product_specs` — offers and their specs; specs keep
  `source_id`, `source_snippet` (verbatim quote) and `confidence`.
* `matches` / `match_details` — verdicts, scores, `score_breakdown` JSONB and
  per-requirement explanations.
* `reports` — generated files with size and SHA-256.
* `api_usage` — every Keenable/LLM/OCR call with credits, tokens and cost.

## Numeric handling

Money, credits, scores and confidences are `NUMERIC` in PostgreSQL. Rust code
casts them to `::float8` on read (SQLx maps NUMERIC to `Decimal`, not `f64`) and
binds `f64` on write (PostgreSQL applies the assignment cast). Python workers
receive `::float8` values as `float` simply by casting in the query.

## Invariants

* `documents.sha256` + `tender_id` unique (no duplicate uploads).
* `search_tasks(run_id, component_id)` unique (re-enqueue is a no-op).
* `search_results(task_id, url)` unique (result dedup across queries).
* `sources(original_url, content_hash)` unique (same page text is stored once).
* `products.content_hash` unique (hash of canonical URL, name and model).
* `matches(run_id, component_id, product_id)` unique.

## Inspection

```bash
docker compose -f deploy/docker-compose.yml exec postgres \
  psql -U tendersourcing -d tendersourcing -c "SELECT status, count(*) FROM tenders GROUP BY 1;"

docker compose -f deploy/docker-compose.yml exec postgres \
  psql -U tendersourcing -d tendersourcing \
  -c "SELECT service, count(*), sum(credits) FROM api_usage GROUP BY 1;"
```
