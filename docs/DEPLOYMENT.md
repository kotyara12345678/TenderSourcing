# Deployment

## Prerequisites

* Docker Engine 24+ with Compose v2.
* A `.env` file at the repository root (copy `.env.example`).

## Steps

```bash
cp .env.example .env
# required: APP_SECRET, AUTH_SECRET, AUTH_USERNAME, AUTH_PASSWORD,
#           POSTGRES_PASSWORD, KEENABLE_API_KEY, LLM_API_KEY
docker compose -f deploy/docker-compose.yml --env-file .env up -d --build
docker compose -f deploy/docker-compose.yml --env-file .env ps
```

The UI is served by nginx on `http://localhost:${FRONTEND_PORT:-8088}`; nginx
proxies `/api/` (including WebSockets) to the API gateway. Only the frontend
port is published — PostgreSQL, Redis, gRPC endpoints (`search:50052`,
`matching:50051`) and worker health ports stay inside the Compose network.

## Services

| Service | Image | Purpose |
| --- | --- | --- |
| `postgres` | postgres:16-alpine | data, `pgdata` volume |
| `redis` | redis:7-alpine | streams + Pub/Sub, `redisdata` volume |
| `api` | Dockerfile.rust (`tender-api`) | HTTP API, migrations, operator bootstrap |
| `orchestrator` | Dockerfile.rust | FSM driver, watchdog |
| `search` | Dockerfile.rust | Keenable client + `jobs:search` worker |
| `matching` | Dockerfile.rust | matcher/ranker gRPC |
| `document-worker` | Dockerfile.worker | parsing, `storage` volume |
| `ai-worker` | Dockerfile.worker | LLM extraction |
| `report-worker` | Dockerfile.worker | XLSX/PDF/DOCX, `storage` volume |
| `frontend` | Dockerfile.frontend | nginx + SPA |

The `storage` volume is shared by `api` (writes uploads), `document-worker`
(reads them) and `report-worker` (writes reports); the API streams downloads.

## Health checks

* `/healthz` — process alive (API on `:8080`, workers on `HTTP_PORT=8090`).
* `/readyz` — PostgreSQL + Redis reachable; the search service also probes
  Keenable through the free public fetch endpoint.

## First login

The operator account is created from `AUTH_USERNAME`/`AUTH_PASSWORD` on API
start. Changing `AUTH_PASSWORD` in ENV rotates the stored argon2id hash on the
next start and revokes all sessions (`password_fingerprint` comparison).

## Upgrades

```bash
git pull
docker compose -f deploy/docker-compose.yml --env-file .env build
docker compose -f deploy/docker-compose.yml --env-file .env up -d
```

Migrations run automatically at API start; they are additive and idempotent.
Rollback = redeploy the previous image tag; schema changes are never destructive
in v1.

## Backups

```bash
docker compose -f deploy/docker-compose.yml --env-file .env exec -T postgres \
  pg_dump -U tendersourcing tendersourcing | gzip > backup-$(date +%F).sql.gz
```

Back up the `storage` volume too (documents and generated reports) — the database
stores only keys.

## Production hardening checklist

* `APP_ENV=production`, `AUTH_COOKIE_SECURE=true`, real `APP_SECRET`/`AUTH_SECRET`.
* TLS terminated in front of nginx (reverse proxy or a TLS-enabled nginx config).
* `PUBLIC_API_ORIGIN` = your real origin (validated: no `localhost` in production).
* `KEENABLE_MAX_RPS ≤ 10`; keep 8 for headroom.
* Restrict outbound traffic to `api.keenable.ai`, your LLM provider and your OCR
  provider (defence in depth for the direct fetch backend).
* Set `SENTRY_DSN` only if you actually run Sentry; otherwise leave empty.
