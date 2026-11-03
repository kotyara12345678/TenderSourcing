"""Requirement extraction: prompt, provenance validation, persistence.

The worker replaces the document's previously extracted positions inside one
transaction, so a retried job never duplicates components.
"""

from __future__ import annotations

import json
import logging
import uuid

import asyncpg

from common.config import Config
from generated.ai.v1 import ai_pb2
from generated.common.v1 import common_pb2

from .llm import LlmClient
from .schemas import RequirementsExtraction, snippet_is_verbatim

LOGGER = logging.getLogger(__name__)

MAX_DOCUMENT_CHARS = 60_000

SYSTEM_PROMPT = """You are a procurement engineer extracting tender positions and their technical requirements.

Return ONLY a JSON object: {"components": [{"name": "...", "quantity": 1, "unit": "шт",
"category": "...", "search_hints": {"alternate_names": [], "english_name": "", "manufacturer": "",
"model": "", "suggested_queries": []}, "requirements": [{"name": "...", "value_text": "...",
"value_num": null, "unit": "", "data_type": "STRING", "mandatory": false, "range_min": null,
"range_max": null, "tolerance": null, "allowed_values": [], "confidence": 0.8,
"source_snippet": "..."}]}]}

Rules:
1. One component per tender position; name it exactly as the document does.
2. Every requirement MUST include source_snippet: an exact substring of the document
   text (same words, same digits) that proves the value. If you cannot quote it, omit
   the requirement.
3. Never invent values. If a value is not stated, omit that requirement entirely.
4. data_type is one of: STRING, NUMBER, RANGE, BOOLEAN, ENUM, STANDARD, MATERIAL,
   MODEL, DIMENSION.
5. mandatory=true only when the document requires the value ("должен", "не менее",
   "не более", "обязательно", "*"). Otherwise false.
6. quantity defaults to 1; unit defaults to "шт"; category is a short product category.
7. Keep requirement names short and canonical (e.g. "Материал", "Мощность", "Напряжение")."""


def build_user_prompt(document_text: str) -> str:
    truncated = document_text[:MAX_DOCUMENT_CHARS]
    note = ""
    if len(document_text) > MAX_DOCUMENT_CHARS:
        note = (
            "\n\nNOTE: the document was truncated for this request; extract only what is "
            "visible in the text above."
        )
    return (
        "DOCUMENT TEXT (verbatim, pages separated by markers):\n"
        f"{truncated}{note}\n\n"
        "Extract every tender position with its technical requirements as JSON."
    )


async def llm_budget_exhausted(
    pool: asyncpg.Pool, cfg: Config, tender_id: uuid.UUID
) -> bool:
    async with pool.acquire() as connection:
        used = await connection.fetchval(
            "SELECT COALESCE(SUM(tokens_in + tokens_out), 0)::bigint FROM api_usage "
            "WHERE tender_id = $1 AND service = 'llm'",
            tender_id,
        )
    return int(used or 0) >= cfg.max_llm_tokens_per_tender


async def record_usage(
    pool: asyncpg.Pool,
    *,
    model: str,
    tender_id: uuid.UUID,
    task_id: str,
    tokens_in: int,
    tokens_out: int,
) -> None:
    async with pool.acquire() as connection:
        await connection.execute(
            """INSERT INTO api_usage (service, sku, tender_id, task_id, quantity, tokens_in, tokens_out)
               VALUES ('llm', $1, $2, $3, 1, $4, $5)""",
            model,
            tender_id,
            task_id,
            tokens_in,
            tokens_out,
        )


async def extract(
    llm: LlmClient,
    document_text: str,
    escalate: bool,
) -> tuple[RequirementsExtraction, int, int]:
    payload, tokens_in, tokens_out = await llm.complete_json(
        system=SYSTEM_PROMPT,
        user=build_user_prompt(document_text),
        escalate=escalate,
    )
    extraction = RequirementsExtraction.model_validate(payload)
    return extraction, tokens_in, tokens_out


async def persist(
    pool: asyncpg.Pool,
    *,
    tender_id: uuid.UUID,
    document_id: uuid.UUID,
    extraction: RequirementsExtraction,
    document_text: str,
    max_components: int,
) -> tuple[int, int]:
    """Replace this document's extracted positions in one transaction."""
    valid_components = []
    for component in extraction.components:
        if not component.name.strip():
            continue
        kept_requirements = []
        for requirement in component.requirements:
            if not snippet_is_verbatim(requirement.source_snippet, document_text):
                LOGGER.warning(
                    "dropping requirement '%s' of '%s': snippet is not verbatim",
                    requirement.name,
                    component.name,
                )
                continue
            kept_requirements.append(requirement)
        component.requirements = kept_requirements
        valid_components.append(component)

    if len(valid_components) > max_components:
        LOGGER.warning(
            "document %s produced %s positions; capping at %s",
            document_id,
            len(valid_components),
            max_components,
        )
        valid_components = valid_components[:max_components]

    components_created = 0
    requirements_created = 0

    async with pool.acquire() as connection:
        async with connection.transaction():
            await connection.execute(
                "DELETE FROM components WHERE tender_id = $1 AND document_id = $2 AND source = 'document'",
                tender_id,
                document_id,
            )
            position_no = int(
                await connection.fetchval(
                    "SELECT COALESCE(MAX(position_no), 0) FROM components WHERE tender_id = $1",
                    tender_id,
                )
                or 0
            )
            for component in valid_components:
                position_no += 1
                component_id = await connection.fetchval(
                    """INSERT INTO components
                           (tender_id, position_no, name, quantity, unit, category, status,
                            source, document_id, search_hints)
                       VALUES ($1, $2, $3, $4::float8, $5, $6, 'EXTRACTED', 'document', $7, $8::jsonb)
                       RETURNING id""",
                    tender_id,
                    position_no,
                    component.name.strip(),
                    component.quantity,
                    component.unit.strip() or "шт",
                    component.category,
                    document_id,
                    json.dumps(component.search_hints or {}, ensure_ascii=False),
                )
                components_created += 1

                for requirement in component.requirements:
                    await connection.execute(
                        """INSERT INTO requirements
                               (component_id, name, value_text, value_num, unit, data_type,
                                mandatory, range_min, range_max, tolerance, allowed_values,
                                source, confidence)
                           VALUES ($1, $2, $3, $4::float8, $5, $6, $7, $8::float8,
                                   $9::float8, $10::float8, $11::jsonb, 'document', $12::float8)""",
                        component_id,
                        requirement.name.strip(),
                        requirement.value_text.strip(),
                        requirement.value_num,
                        requirement.unit.strip(),
                        requirement.data_type,
                        requirement.mandatory,
                        requirement.range_min,
                        requirement.range_max,
                        requirement.tolerance,
                        json.dumps(requirement.allowed_values, ensure_ascii=False),
                        requirement.confidence,
                    )
                    requirements_created += 1

    return components_created, requirements_created


def notice(
    envelope: common_pb2.JobEnvelope | None,
    document_id: str,
    components_created: int,
    requirements_created: int,
    success: bool,
    error_message: str = "",
) -> ai_pb2.RequirementsExtractedNotice:
    return ai_pb2.RequirementsExtractedNotice(
        envelope=envelope,
        document_id=document_id,
        components_created=components_created,
        requirements_created=requirements_created,
        success=success,
        error_message=error_message[:2000],
    )
