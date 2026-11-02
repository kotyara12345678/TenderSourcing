-- ============================================================================
-- Tender positions (components) and their technical requirements.
-- ============================================================================

CREATE TABLE components (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tender_id       UUID NOT NULL REFERENCES tenders(id) ON DELETE CASCADE,
    position_no     INT NOT NULL,
    name            TEXT NOT NULL,
    quantity        NUMERIC(18,3) NOT NULL DEFAULT 1 CHECK (quantity > 0),
    unit            TEXT NOT NULL DEFAULT 'pcs',
    category        TEXT NOT NULL DEFAULT '',
    status          TEXT NOT NULL DEFAULT 'DRAFT'
        CHECK (status IN ('DRAFT','EXTRACTED','REVIEWED','QUEUED','SEARCHING',
                          'SEARCHED','MATCHED','FAILED','SKIPPED')),
    source          TEXT NOT NULL DEFAULT 'document' CHECK (source IN ('document','manual')),
    document_id     UUID REFERENCES documents(id) ON DELETE SET NULL,
    -- Query-generation hints produced by the AI worker:
    -- {"alternate_names":[], "english_name":"", "manufacturer":"", "model":"",
    --  "suggested_queries":[]}
    search_hints    JSONB NOT NULL DEFAULT '{}',
    error_message   TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tender_id, position_no)
);
CREATE INDEX idx_components_tender  ON components(tender_id, position_no);
CREATE INDEX idx_components_status  ON components(tender_id, status);

-- One requirement = one checkable technical constraint.
-- Unknown values are stored as empty/NULL — never as fake matches.
CREATE TABLE requirements (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    component_id    UUID NOT NULL REFERENCES components(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    value_text      TEXT NOT NULL DEFAULT '',
    value_num       NUMERIC(18,6),
    unit            TEXT NOT NULL DEFAULT '',
    data_type       TEXT NOT NULL DEFAULT 'STRING'
        CHECK (data_type IN ('STRING','NUMBER','RANGE','BOOLEAN','ENUM',
                             'STANDARD','MATERIAL','MODEL','DIMENSION')),
    mandatory       BOOLEAN NOT NULL DEFAULT FALSE,
    range_min       NUMERIC(18,6),
    range_max       NUMERIC(18,6),
    tolerance       NUMERIC(8,6) CHECK (tolerance IS NULL OR (tolerance >= 0 AND tolerance <= 1)),
    allowed_values  JSONB NOT NULL DEFAULT '[]',   -- ENUM: acceptable values
    source          TEXT NOT NULL DEFAULT 'document' CHECK (source IN ('document','manual')),
    confidence      NUMERIC(4,3) NOT NULL DEFAULT 1.000
        CHECK (confidence >= 0 AND confidence <= 1),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_requirements_component ON requirements(component_id);

CREATE TRIGGER trg_components_updated  BEFORE UPDATE ON components   FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_requirements_updated BEFORE UPDATE ON requirements FOR EACH ROW EXECUTE FUNCTION set_updated_at();
