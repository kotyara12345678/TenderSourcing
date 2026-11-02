-- ============================================================================
-- Search runs, queries/results, sources, suppliers, products, matches,
-- reports and API usage accounting.
-- ============================================================================

-- ---------------------------------------------------------------------------
-- A search run = one attempt to process a whole tender through the pipeline.
-- ---------------------------------------------------------------------------
CREATE TABLE search_runs (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tender_id           UUID NOT NULL REFERENCES tenders(id) ON DELETE CASCADE,
    status              TEXT NOT NULL DEFAULT 'QUEUED'
        CHECK (status IN ('QUEUED','RUNNING','COMPLETED','COMPLETED_WITH_ERRORS',
                          'FAILED','CANCELLED')),
    triggered_by        UUID REFERENCES users(id) ON DELETE SET NULL,
    started_at          TIMESTAMPTZ,
    finished_at         TIMESTAMPTZ,
    cancelled_at        TIMESTAMPTZ,
    components_total    INT NOT NULL DEFAULT 0,
    components_done     INT NOT NULL DEFAULT 0,
    components_failed   INT NOT NULL DEFAULT 0,
    queries_issued      INT NOT NULL DEFAULT 0,
    results_found       INT NOT NULL DEFAULT 0,
    pages_fetched       INT NOT NULL DEFAULT 0,
    search_credits      BIGINT NOT NULL DEFAULT 0,  -- Keenable usage units
    fetch_credits       BIGINT NOT NULL DEFAULT 0,
    error_message       TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_runs_tender  ON search_runs(tender_id, created_at DESC);
CREATE INDEX idx_runs_active  ON search_runs(status) WHERE status IN ('QUEUED','RUNNING');

-- Per-position task within a run. UNIQUE(run_id, component_id) is the
-- idempotency guard: re-enqueueing the same position is a no-op.
CREATE TABLE search_tasks (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id          UUID NOT NULL REFERENCES search_runs(id) ON DELETE CASCADE,
    component_id    UUID NOT NULL REFERENCES components(id) ON DELETE CASCADE,
    status          TEXT NOT NULL DEFAULT 'QUEUED'
        CHECK (status IN ('QUEUED','SEARCHING','FETCHING','EXTRACTING',
                          'MATCHING','DONE','FAILED','CANCELLED')),
    attempts        INT NOT NULL DEFAULT 0,
    last_error      TEXT,
    started_at      TIMESTAMPTZ,
    finished_at     TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (run_id, component_id)
);
CREATE INDEX idx_search_tasks_run    ON search_tasks(run_id, status);
CREATE INDEX idx_search_tasks_active ON search_tasks(status)
    WHERE status IN ('QUEUED','SEARCHING','FETCHING','EXTRACTING','MATCHING');

CREATE TABLE search_queries (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    task_id         UUID NOT NULL REFERENCES search_tasks(id) ON DELETE CASCADE,
    query           TEXT NOT NULL,
    kind            TEXT NOT NULL DEFAULT 'custom'
        CHECK (kind IN ('exact','alt','manufacturer','model','specs',
                        'synonym','multilingual','custom')),
    sort_order      INT NOT NULL DEFAULT 0,
    results_count   INT NOT NULL DEFAULT 0,
    error           TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (task_id, query)
);
CREATE INDEX idx_search_queries_task ON search_queries(task_id);

-- External pages with full provenance (spec §15).
CREATE TABLE sources (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    url             TEXT NOT NULL,             -- final URL (after redirects)
    original_url    TEXT NOT NULL,             -- URL as submitted/found
    provider        TEXT NOT NULL DEFAULT 'keenable'
        CHECK (provider IN ('keenable','direct')),
    retrieved_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    content_hash    TEXT NOT NULL,
    http_status     INT,
    content_type    TEXT,
    title           TEXT NOT NULL DEFAULT '',
    content         TEXT NOT NULL DEFAULT '',  -- extracted text/markdown, size-capped
    snippet         TEXT NOT NULL DEFAULT '',
    extracted_data  JSONB NOT NULL DEFAULT '{}',
    size_bytes      INT NOT NULL DEFAULT 0,
    UNIQUE (original_url, content_hash)
);
CREATE INDEX idx_sources_retrieved ON sources(retrieved_at DESC);
CREATE INDEX idx_sources_hash      ON sources(content_hash);

CREATE TABLE search_results (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    query_id        UUID NOT NULL REFERENCES search_queries(id) ON DELETE CASCADE,
    task_id         UUID NOT NULL REFERENCES search_tasks(id) ON DELETE CASCADE,
    url             TEXT NOT NULL,             -- deduplicated within a task
    title           TEXT NOT NULL DEFAULT '',
    snippet         TEXT NOT NULL DEFAULT '',
    published_at    TIMESTAMPTZ,
    acquired_at     TIMESTAMPTZ,
    rank            INT NOT NULL DEFAULT 0,
    relevance_score NUMERIC(5,4),              -- NULL until prefiltered
    fetched         BOOLEAN NOT NULL DEFAULT FALSE,
    source_id       UUID REFERENCES sources(id) ON DELETE SET NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (task_id, url)
);
CREATE INDEX idx_search_results_task ON search_results(task_id, relevance_score DESC NULLS LAST);

-- ---------------------------------------------------------------------------
-- Suppliers and products. A product may match many components (matches table
-- below); a position has many candidate products.
-- reliability_score is NULL when unknown — never invented.
-- ---------------------------------------------------------------------------
CREATE TABLE suppliers (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name              TEXT NOT NULL,
    domain            TEXT NOT NULL DEFAULT '',
    url               TEXT NOT NULL DEFAULT '',
    reliability_score NUMERIC(4,3)
        CHECK (reliability_score IS NULL OR (reliability_score >= 0 AND reliability_score <= 1)),
    notes             TEXT NOT NULL DEFAULT '',
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (name, domain)
);
CREATE INDEX idx_suppliers_domain ON suppliers(domain);

CREATE TABLE products (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name            TEXT NOT NULL,
    manufacturer    TEXT NOT NULL DEFAULT '',
    model           TEXT NOT NULL DEFAULT '',
    supplier_id     UUID REFERENCES suppliers(id) ON DELETE SET NULL,
    price           NUMERIC(18,4) CHECK (price IS NULL OR price >= 0),  -- NULL = unknown
    currency        TEXT,                                              -- NULL = unknown
    lead_time_days  INT CHECK (lead_time_days IS NULL OR lead_time_days >= 0),
    canonical_url   TEXT NOT NULL DEFAULT '',
    source_id       UUID REFERENCES sources(id) ON DELETE SET NULL,
    confidence      NUMERIC(4,3) NOT NULL DEFAULT 0.500,
    status          TEXT NOT NULL DEFAULT 'CANDIDATE'
        CHECK (status IN ('CANDIDATE','VERIFIED','REJECTED')),
    content_hash    TEXT NOT NULL,   -- dedup key: hash(canonical_url|name|model)
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (content_hash)
);
CREATE INDEX idx_products_supplier    ON products(supplier_id);
CREATE INDEX idx_products_manufacturer ON products(manufacturer);

-- Every spec value keeps provenance: source_id + verbatim snippet + confidence.
CREATE TABLE product_specs (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    product_id      UUID NOT NULL REFERENCES products(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    value_text      TEXT NOT NULL DEFAULT '',
    value_num       NUMERIC(18,6),
    unit            TEXT NOT NULL DEFAULT '',
    data_type       TEXT NOT NULL DEFAULT 'STRING',
    source_id       UUID REFERENCES sources(id) ON DELETE SET NULL,
    source_snippet  TEXT NOT NULL DEFAULT '',
    confidence      NUMERIC(4,3) NOT NULL DEFAULT 0.500,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (product_id, name)
);
CREATE INDEX idx_product_specs_product ON product_specs(product_id);

-- ---------------------------------------------------------------------------
-- Matching + ranking results (one row per run × position × product).
-- ---------------------------------------------------------------------------
CREATE TABLE matches (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    component_id     UUID NOT NULL REFERENCES components(id) ON DELETE CASCADE,
    product_id       UUID NOT NULL REFERENCES products(id) ON DELETE CASCADE,
    run_id           UUID NOT NULL REFERENCES search_runs(id) ON DELETE CASCADE,
    verdict          TEXT NOT NULL
        CHECK (verdict IN ('FULL','TOLERANCE','PARTIAL','MISMATCH','INSUFFICIENT')),
    eligible         BOOLEAN NOT NULL DEFAULT FALSE,
    technical_score  NUMERIC(5,4) NOT NULL DEFAULT 0,
    rank             INT,                       -- NULL until ranked
    total_score      NUMERIC(5,4),              -- NULL until ranked
    score_breakdown  JSONB NOT NULL DEFAULT '[]', -- ScoreComponent[]
    explanation      TEXT NOT NULL DEFAULT '',
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (run_id, component_id, product_id)
);
CREATE INDEX idx_matches_component ON matches(component_id, eligible, total_score DESC NULLS LAST);
CREATE INDEX idx_matches_run       ON matches(run_id);
CREATE INDEX idx_matches_product   ON matches(product_id);

-- Per-requirement check with human-readable explanation of every mismatch.
CREATE TABLE match_details (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    match_id         UUID NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
    requirement_id   UUID REFERENCES requirements(id) ON DELETE SET NULL,
    requirement_name TEXT NOT NULL,
    status           TEXT NOT NULL
        CHECK (status IN ('match','tolerance','partial','mismatch','missing')),
    expected         TEXT NOT NULL DEFAULT '',
    actual           TEXT NOT NULL DEFAULT '',
    unit             TEXT NOT NULL DEFAULT '',
    explanation      TEXT NOT NULL DEFAULT '',
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_match_details_match ON match_details(match_id);

-- ---------------------------------------------------------------------------
-- Reports (files in object storage).
-- ---------------------------------------------------------------------------
CREATE TABLE reports (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tender_id       UUID NOT NULL REFERENCES tenders(id) ON DELETE CASCADE,
    run_id          UUID REFERENCES search_runs(id) ON DELETE SET NULL,
    format          TEXT NOT NULL CHECK (format IN ('xlsx','pdf','docx')),
    storage_key     TEXT NOT NULL,
    size_bytes      BIGINT NOT NULL DEFAULT 0,
    sha256          TEXT NOT NULL DEFAULT '',
    status          TEXT NOT NULL DEFAULT 'PENDING'
        CHECK (status IN ('PENDING','READY','FAILED')),
    error_message   TEXT,
    created_by      UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    ready_at        TIMESTAMPTZ
);
CREATE INDEX idx_reports_tender ON reports(tender_id, created_at DESC);

-- ---------------------------------------------------------------------------
-- External API accounting: every Keenable/LLM/OCR call is recorded so the
-- operator can see spend per tender and per position.
-- ---------------------------------------------------------------------------
CREATE TABLE api_usage (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    service       TEXT NOT NULL
        CHECK (service IN ('keenable_search','keenable_fetch','llm','ocr')),
    sku           TEXT NOT NULL DEFAULT '',   -- "search.pro", "fetch.live", model name
    tender_id     UUID REFERENCES tenders(id) ON DELETE SET NULL,
    run_id        UUID REFERENCES search_runs(id) ON DELETE SET NULL,
    component_id  UUID REFERENCES components(id) ON DELETE SET NULL,
    task_id       TEXT NOT NULL DEFAULT '',
    quantity      BIGINT NOT NULL DEFAULT 1,
    credits       NUMERIC(12,4) NOT NULL DEFAULT 0,
    tokens_in     BIGINT NOT NULL DEFAULT 0,
    tokens_out    BIGINT NOT NULL DEFAULT 0,
    cost_usd      NUMERIC(12,6) NOT NULL DEFAULT 0,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_api_usage_tender   ON api_usage(tender_id, created_at DESC);
CREATE INDEX idx_api_usage_service  ON api_usage(service, created_at DESC);
CREATE INDEX idx_api_usage_run      ON api_usage(run_id);

CREATE TRIGGER trg_search_runs_updated  BEFORE UPDATE ON search_runs  FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_search_tasks_updated BEFORE UPDATE ON search_tasks FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_suppliers_updated    BEFORE UPDATE ON suppliers    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_products_updated     BEFORE UPDATE ON products     FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_matches_updated      BEFORE UPDATE ON matches      FOR EACH ROW EXECUTE FUNCTION set_updated_at();
