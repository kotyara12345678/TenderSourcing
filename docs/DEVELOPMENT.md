# Development

## Toolchain

| Tool | Version | Notes |
| --- | --- | --- |
| Rust | stable | `rustfmt`, `clippy`; `protobuf-compiler` for build scripts |
| Python | 3.12 | `grpcio-tools` for protobuf generation |
| Node.js | 22 | Vite 6 |
| PostgreSQL | 16 | local or Compose |
| Redis | 7 | local or Compose |

## Layout

```
services/rust/Cargo.toml        # workspace: common, api, orchestrator, search, matching
services/python/                # common/, document_worker/, ai_worker/, report_worker/
services/frontend/              # Vite + React + TS (strict)
proto/                          # contracts (generated code is never committed)
migrations/                     # SQLx migrations (applied by the API only)
deploy/                         # compose, Dockerfiles, nginx
scripts/                        # gen-proto-python.sh, clean-artifacts.sh
```

## Local run

```bash
# 1. infrastructure
docker compose -f deploy/docker-compose.yml up -d postgres redis

# 2. API (runs migrations and bootstraps the operator)
cd services/rust
cargo run -p tender-api

# 3. Rust workers (separate terminals)
cargo run -p tender-orchestrator
cargo run -p tender-search
cargo run -p tender-matching

# 4. Python workers
bash scripts/gen-proto-python.sh
cd services/python
PYTHONPATH=. python -m document_worker.main
PYTHONPATH=. python -m ai_worker.main
PYTHONPATH=. python -m report_worker.main

# 5. frontend (proxies /api to http://localhost:8080)
cd services/frontend
npm install
npm run dev
```

## Conventions

* Rust: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`
  must stay clean. Errors go through `AppError`; internal details are logged and
  replaced with a generic message.
* Protobuf is the only contract source: add a field, regenerate, update both
  Rust and Python code paths.
* Python: type hints everywhere, `dataclasses`/Pydantic v2, no bare `except`;
  failures either persist a real failure or release the dedup key for retry.
* Frontend: TypeScript strict with `noUncheckedIndexedAccess`; all API access
  through `src/api/endpoints.ts`; no `dangerouslySetInnerHTML`.
* Every new behaviour gets a unit test where the logic is pure (FSM, matcher,
  ranking, query generation, SSRF checks, parsers).

## Checks

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

python -m compileall services/python

cd services/frontend && npm run typecheck && npm run build
```

CI runs exactly these (`.github/workflows/ci.yml`); image builds live in
`.github/workflows/docker.yml`.

## Artifacts

Never commit generated protobuf code, `target/`, `node_modules/`, `dist/` or
`services/python/generated/`. `scripts/clean-artifacts.sh --dry-run` shows what
would be removed; the script only deletes generated artifacts.
