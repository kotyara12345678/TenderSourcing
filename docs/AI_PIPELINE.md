# AI pipeline

Two Python consumers, both using an OpenAI-compatible chat endpoint
(`LLM_BASE_URL`, `LLM_MODEL`, optional `LLM_FALLBACK_MODEL`), with strict JSON
validation and provenance checks. Temperature is 0 by default
(`LLM_TEMPERATURE`) — extraction must be reproducible.

## Requirement extraction (`jobs:ai.requirements`)

Input: `document_pages.content` for one document (pages joined with markers,
capped at 60 000 characters; truncation is stated in the prompt).

Output schema (Pydantic, mirrors `proto/ai/v1/ai.proto`):

```
{"components": [{name, quantity, unit, category, search_hints,
                 requirements: [{name, value_text, value_num, unit, data_type,
                                 mandatory, range_min, range_max, tolerance,
                                 allowed_values, confidence, source_snippet}]}]}
```

Rules enforced after parsing:

* `source_snippet` must be a verbatim (whitespace-normalized, ё→е) substring of
  the document text — otherwise the requirement is **dropped** and logged.
* `data_type` outside the allowed set degrades to `STRING`;
  `confidence` is clamped to 0..1.
* Positions exceeding `MAX_TENDER_COMPONENTS` are capped (logged).

Persistence is transactional: previous positions extracted from the same
document (`source='document'`, `document_id=…`) are deleted first, so retries
never duplicate data. `documents.ai_status` moves `PENDING → RUNNING →
DONE|FAILED`.

## Product extraction (`jobs:ai.products`)

Input: the tender position (name + requirements), plus up to eight fetched
`sources` for that task (12 000 characters each, ordered by relevance).
Each source is presented as `SOURCE <uuid>` and the model must attribute every
spec to one of them.

Validation:

* `spec.source_id` must reference a provided source;
* `spec.source_snippet` must be a verbatim substring of **that** source content;
* products without a single verified spec are dropped entirely;
* `price`, `currency`, `lead_time_days` stay `null` unless the page states them.

Persistence: suppliers upserted on `(name, domain)`, products on
`content_hash = sha256(canonical_url|name|model)`, specs on `(product_id, name)`.

## Token budget

Every call records `api_usage(service='llm', sku=model, tokens_in, tokens_out)`.
Before each call the worker sums the tender's tokens and refuses to continue past
`MAX_LLM_TOKENS_PER_TENDER` (default 2 000 000) — the position is marked failed
with an explicit reason instead of silently burning budget.

## Failure behaviour

* LLM transport/JSON failures: two attempts, then the job fails with a stored
  reason and a failure notice (`success=false`); the orchestrator marks the
  position failed and the run continues.
* `escalate` (in the job) selects `LLM_FALLBACK_MODEL` when configured.
* The model prompt forbids inventing values; the validator enforces it. A value
  that cannot be quoted does not exist for this system.

## Assumptions

* Documents without a text layer need `OCR_PROVIDER=api`; otherwise their pages
  carry no text and extraction yields nothing (reported, not fabricated).
* DOCX has no page concept: paragraphs/tables are grouped into virtual pages of
  40 blocks (`DOCX_BLOCKS_PER_PAGE`) purely for provenance.
* XLSX maps one sheet to one page; formula cells are read as values.
