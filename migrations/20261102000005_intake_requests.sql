-- ============================================================================
-- AI intake: a free-form purchasing request ("нужно 200 м кабеля ...") and the
-- structured positions the AI worker derives from it.
--
-- The row is created by the API before the worker job is queued, so the
-- browser can poll one id from the first millisecond: QUEUED -> RUNNING ->
-- COMPLETED | FAILED, then APPLIED once a tender has been materialised.
-- `steps` is the agent trace shown in the UI (append-only, one entry per
-- stage), `progress` is 0..1 so the client never has to guess.
-- ============================================================================

CREATE TABLE intake_requests (
    id              UUID PRIMARY KEY,
    user_id         UUID REFERENCES users(id) ON DELETE SET NULL,
    request_text    TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'QUEUED'
        CHECK (status IN ('QUEUED','RUNNING','COMPLETED','FAILED','APPLIED')),
    progress        REAL NOT NULL DEFAULT 0 CHECK (progress >= 0 AND progress <= 1),
    steps           JSONB NOT NULL DEFAULT '[]',
    -- Validated model output:
    -- {"tender_name":"...","summary":"...","positions":[{"name":"...",
    --   "quantity":10,"unit":"шт","category":"...",
    --   "requirements":[{"name":"...","value_text":"...","value_num":null,
    --                    "unit":"","data_type":"STRING","mandatory":false}]}]}
    result          JSONB,
    error_message   TEXT,
    tender_id       UUID REFERENCES tenders(id) ON DELETE SET NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_intake_requests_user ON intake_requests(user_id, created_at DESC);

CREATE TRIGGER trg_intake_requests_updated BEFORE UPDATE ON intake_requests
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
