-- ============================================================================
-- Core: users, sessions, tenders, documents, pages, activity, settings.
-- Owned by the Rust API Gateway (sqlx::migrate! runs these at startup).
-- ============================================================================

-- ---------------------------------------------------------------------------
-- Auth: single operator account, created from ENV on first start.
-- password_fingerprint = HMAC-SHA256(APP_SECRET, AUTH_PASSWORD) — lets the
-- service detect an ENV password rotation without storing the password and
-- without re-hashing on every boot.
-- ---------------------------------------------------------------------------
CREATE TABLE users (
    id                      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    username                TEXT NOT NULL UNIQUE,
    password_hash           TEXT NOT NULL,                -- argon2id encoded
    password_fingerprint    TEXT NOT NULL DEFAULT '',
    display_name            TEXT NOT NULL DEFAULT '',
    is_active               BOOLEAN NOT NULL DEFAULT TRUE,
    created_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at              TIMESTAMPTZ
);

-- HttpOnly session cookie holds a random token; only its SHA-256 is stored.
CREATE TABLE sessions (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id         UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash      TEXT NOT NULL UNIQUE,
    csrf_token      TEXT NOT NULL,
    user_agent      TEXT NOT NULL DEFAULT '',
    ip              INET,
    expires_at      TIMESTAMPTZ NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    revoked_at      TIMESTAMPTZ
);
CREATE INDEX idx_sessions_user       ON sessions(user_id);
CREATE INDEX idx_sessions_expires_at ON sessions(expires_at) WHERE revoked_at IS NULL;

-- ---------------------------------------------------------------------------
-- Tender + FSM state (see docs/ARCHITECTURE.md for the state diagram).
-- COMPLETED_WITH_ERRORS = partial success: some positions failed, report
-- still generated from the successful ones.
-- ---------------------------------------------------------------------------
CREATE TABLE tenders (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name            TEXT NOT NULL,
    description     TEXT NOT NULL DEFAULT '',
    status          TEXT NOT NULL DEFAULT 'CREATED'
        CHECK (status IN (
            'CREATED','UPLOADING','DOCUMENT_PROCESSING','REQUIREMENT_EXTRACTION',
            'REQUIREMENTS_REVIEW','SEARCH_QUEUED','SEARCHING','FETCHING',
            'PRODUCT_EXTRACTION','MATCHING','RANKING','REPORT_GENERATION',
            'COMPLETED','COMPLETED_WITH_ERRORS','FAILED','CANCELLED'
        )),
    error_message   TEXT,
    created_by      UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at      TIMESTAMPTZ
);
CREATE INDEX idx_tenders_status    ON tenders(status) WHERE deleted_at IS NULL;
CREATE INDEX idx_tenders_created   ON tenders(created_at DESC) WHERE deleted_at IS NULL;

-- ---------------------------------------------------------------------------
-- Uploaded files. Large blobs live in object storage (storage_key), never here.
-- sha256 deduplicates identical uploads within one tender.
-- ---------------------------------------------------------------------------
CREATE TABLE documents (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tender_id           UUID NOT NULL REFERENCES tenders(id) ON DELETE CASCADE,
    original_filename   TEXT NOT NULL,
    storage_key         TEXT NOT NULL,
    mime_type           TEXT NOT NULL,
    size_bytes          BIGINT NOT NULL CHECK (size_bytes > 0),
    sha256              TEXT NOT NULL,
    status              TEXT NOT NULL DEFAULT 'UPLOADED'
        CHECK (status IN ('UPLOADED','QUEUED','PROCESSING','EXTRACTED','FAILED')),
    error_message       TEXT,
    page_count          INT,
    ocr_used            BOOLEAN NOT NULL DEFAULT FALSE,
    uploaded_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    processed_at        TIMESTAMPTZ,
    deleted_at          TIMESTAMPTZ
);
CREATE INDEX idx_documents_tender ON documents(tender_id) WHERE deleted_at IS NULL;
CREATE UNIQUE INDEX uq_documents_tender_sha
    ON documents(tender_id, sha256) WHERE deleted_at IS NULL;

-- Extracted content per page: text + tables + positioned blocks (bbox,
-- confidence) — structure of tables is preserved, positions never mixed.
CREATE TABLE document_pages (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    document_id     UUID NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    page_number     INT NOT NULL CHECK (page_number >= 1),
    content_type    TEXT NOT NULL DEFAULT 'text'
        CHECK (content_type IN ('text','table','image','mixed')),
    content         TEXT NOT NULL DEFAULT '',
    blocks          JSONB NOT NULL DEFAULT '[]',   -- TextBlock[] (bbox, confidence, kind)
    tables          JSONB NOT NULL DEFAULT '[]',   -- Table[]
    ocr_used        BOOLEAN NOT NULL DEFAULT FALSE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (document_id, page_number)
);
CREATE INDEX idx_document_pages_document ON document_pages(document_id, page_number);

-- ---------------------------------------------------------------------------
-- Audit trail of user-visible actions (also powers the Activity tab).
-- ---------------------------------------------------------------------------
CREATE TABLE activity_logs (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tender_id   UUID REFERENCES tenders(id) ON DELETE CASCADE,
    user_id     UUID REFERENCES users(id) ON DELETE SET NULL,
    event_type  TEXT NOT NULL,              -- "tender.created", "search.started", ...
    message     TEXT NOT NULL DEFAULT '',
    metadata    JSONB NOT NULL DEFAULT '{}', -- no secrets, no document contents
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_activity_tender  ON activity_logs(tender_id, created_at DESC);
CREATE INDEX idx_activity_created ON activity_logs(created_at DESC);

-- Operator-adjustable non-secret settings (ranking weights, search prefs).
CREATE TABLE system_settings (
    key         TEXT PRIMARY KEY,
    value       JSONB NOT NULL,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- updated_at maintenance
CREATE OR REPLACE FUNCTION set_updated_at() RETURNS trigger AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_users_updated      BEFORE UPDATE ON users      FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_tenders_updated    BEFORE UPDATE ON tenders    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_settings_updated   BEFORE UPDATE ON system_settings FOR EACH ROW EXECUTE FUNCTION set_updated_at();
