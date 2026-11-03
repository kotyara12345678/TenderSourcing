-- ============================================================================
-- AI-worker markers: documents.ai_status tracks requirements extraction per
-- document so the orchestrator can tell "not processed yet" apart from
-- "processed, produced zero components" without in-memory counters.
-- ============================================================================

ALTER TABLE documents
    ADD COLUMN ai_status TEXT NOT NULL DEFAULT 'PENDING'
        CHECK (ai_status IN ('PENDING', 'RUNNING', 'DONE', 'FAILED'));

-- Only documents that actually reached the AI stage are ever RUNNING/DONE;
-- PENDING on an unparsed document simply means no job was queued yet.
CREATE INDEX idx_documents_ai_status ON documents(tender_id, ai_status)
    WHERE deleted_at IS NULL;
