# TenderSourcing

**Automated sourcing platform for tender documentation.**

Upload tender documents → the platform extracts positions and technical
requirements → searches the web through the Keenable API → fetches candidate
product pages → extracts product specifications → performs *programmatic*
technical matching → ranks eligible offers → produces an Excel/PDF/DOCX
sourcing report ready for a procurement specialist.

> Status: full implementation (all services, workers, SPA, deployment and docs).
> CI validates formatting, clippy, unit tests, protobuf generation, Python
> imports, strict TypeScript and the SQL migrations. Known v1 limitations are
> listed in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Stack

| Layer        | Technology                                                        |
| ------------ | ----------------------------------------------------------------- |
| API Gateway  | Rust · Axum · Tokio · SQLx · WebSocket                            |
| Orchestrator | Rust · Redis Streams · PostgreSQL state machine                   |
| Search       | Rust · Keenable API (`https://keenable.ai`) · SSRF-guarded fetch   |
| Matching     | Rust · deterministic technical matcher + explainable ranking       |
| Documents    | Python 3.12 · FastAPI · PyMuPDF · python-docx · openpyxl · OCR    |
| AI           | Python 3.12 · provider-agnostic LLM client · Pydantic v2           |
| Reports      | Python 3.12 · openpyxl · ReportLab · python-docx                  |
| Frontend     | React · TypeScript (strict) · Vite · Tailwind · TanStack Query     |
| Data         | PostgreSQL · Redis · local/S3-compatible object storage           |
| Transport    | gRPC + Protobuf (`proto/`) between services, Redis Streams for jobs |
| CI/CD        | GitHub Actions · GHCR · Docker Compose · Nginx                    |

## Repository layout

```
TenderSourcing/
├── proto/                  # versioned gRPC/Protobuf contracts (single source)
├── migrations/             # SQLx PostgreSQL migrations (owned by Rust)
├── services/
│   ├── rust/               # Cargo workspace
│   │   └── crates/
│   │       ├── common/     # config, errors, telemetry, domain types
│   │       ├── api/        # API Gateway (Axum)
│   │       ├── orchestrator/
│   │       ├── search/     # Search Engine (Keenable)
│   │       └── matching/   # Matching + Ranking Engine
│   └── python/
│       ├── document_worker/
│       ├── ai_worker/
│       └── report_worker/
│       └── frontend/       # React SPA (Vite + TS strict)
├── deploy/                 # docker-compose, Dockerfiles, nginx
│   ├── docker/
│   └── nginx/
├── scripts/                # repo utilities (clean-artifacts.sh)
├── docs/                   # architecture & operation documentation
└── .github/workflows/      # CI, Docker build, deploy
```

## Quick start (Docker Compose)

```bash
cp .env.example .env        # fill in secrets
docker compose -f deploy/docker-compose.yml up -d --build
# => http://localhost:8088 (nginx serves the SPA and proxies /api)
```

Builds and tests run in GitHub Actions (`.github/workflows/ci.yml`); container
images are built and pushed to GHCR by `.github/workflows/docker.yml`.
Generated artifacts are never committed — cleanup is explicit and controlled:

```bash
scripts/clean-artifacts.sh --dry-run   # show what would be removed
scripts/clean-artifacts.sh             # remove generated artifacts only
```

## Documentation

- [Architecture](docs/ARCHITECTURE.md)
- [API](docs/API.md)
- [gRPC contracts](docs/GRPC.md)
- [Database](docs/DATABASE.md)
- [Search engine (Keenable)](docs/SEARCH_ENGINE.md)
- [AI pipeline](docs/AI_PIPELINE.md)
- [Deployment](docs/DEPLOYMENT.md)
- [Environment](docs/ENVIRONMENT.md)
- [Security](docs/SECURITY.md)
- [Development](docs/DEVELOPMENT.md)
- [Troubleshooting](docs/TROUBLESHOOTING.md)
