"""Product extraction from fetched supplier pages.

Every extracted spec keeps provenance: the page it came from (``source_id``)
and a verbatim ``source_snippet``. Specs whose snippet is not found in the
referenced page are dropped; products without any verified spec are dropped
too, because they could never be matched or justified.
"""

from __future__ import annotations

import hashlib
import json
import logging
import uuid
from urllib.parse import urlsplit

import asyncpg

from common.config import Config
from generated.ai.v1 import ai_pb2
from generated.common.v1 import common_pb2

from .llm import LlmClient
from .schemas import ProductsExtraction, snippet_is_verbatim

LOGGER = logging.getLogger(__name__)

MAX_SOURCES = 8
MAX_SOURCE_CHARS = 12_000

SYSTEM_PROMPT = """You are a procurement analyst extracting concrete product offers from supplier pages.

Return ONLY a JSON object: {"products": [{"name": "...", "manufacturer": "...", "model": "...",
"supplier_name": "...", "supplier_url": "...", "price": null, "currency": "",
"lead_time_days": null, "canonical_url": "...", "source_id": "<uuid>", "confidence": 0.7,
"specs": [{"name": "...", "value_text": "...", "value_num": null, "unit": "",
"data_type": "STRING", "source_id": "<uuid>", "source_snippet": "...", "confidence": 0.8}]}]}

Rules:
1. Only extract products explicitly present in the SOURCE blocks below.
2. Every spec must carry source_id of the SOURCE it came from and source_snippet: an
   exact substring of that source content (same words, same digits).
3. Never guess price, currency or lead time: use null / "" when the page does not state it.
4. Never merge data from different sources into one product; if two pages describe the
   same model, return the better-documented one.
5. data_type is one of: STRING, NUMBER, RANGE, BOOLEAN, ENUM, STANDARD, MATERIAL,
   MODEL, DIMENSION.
6. canonical_url is the product page URL; supplier_url is the supplier/shop page."""


def build_user_prompt(
    component_name: str,
    requirements: list[str],
    sources: list[tuple[str, str, str, str]],
) -> str:
    requirement_list = "\n".join(f"- {name}" for name in requirements) or "- (none)"
    blocks: list[str] = []
    for source_id, url, title, content in sources[:MAX_SOURCES]:
        blocks.append(
            f"SOURCE {source_id}\nURL: {url}\nTITLE: {title}\nCONTENT:\n"
            f"{content[:MAX_SOURCE_CHARS]}"
        )
    return (
        f"TENDER POSITION: {component_name}\n"
        f"REQUIREMENTS TO CHECK:\n{requirement_list}\n\n"
        "SOURCE PAGES:\n\n" + "\n\n".join(blocks) + "\n\n"
        "Extract the products as JSON."
    )


async def extract(
    llm: LlmClient,
    component_name: str,
    requirements: list[str],
    sources: list[tuple[str, str, str, str]],
    escalate: bool,
) -> tuple[ProductsExtraction, int, int]:
    payload, tokens_in, tokens_out = await llm.complete_json(
        system=SYSTEM_PROMPT,
        user=build_user_prompt(component_name, requirements, sources),
        escalate=escalate,
    )
    extraction = ProductsExtraction.model_validate(payload)
    return extraction, tokens_in, tokens_out


def _domain(url: str) -> str:
    try:
        return (urlsplit(url).netloc or "").lower()
    except ValueError:
        return ""


def _content_hash(canonical_url: str, name: str, model: str) -> str:
    raw = f"{canonical_url.strip()}|{name.strip().lower()}|{model.strip().lower()}"
    return hashlib.sha256(raw.encode("utf-8")).hexdigest()


async def persist(
    pool: asyncpg.Pool,
    *,
    extraction: ProductsExtraction,
    sources: dict[str, str],
    max_products: int,
) -> tuple[int, int]:
    """Persist verified products; returns ``(products_created, specs_created)``."""
    products_created = 0
    specs_created = 0

    async with pool.acquire() as connection:
        async with connection.transaction():
            for product in extraction.products[:max_products]:
                kept_specs = []
                for spec in product.specs:
                    source_content = sources.get(spec.source_id.strip())
                    if source_content is None:
                        LOGGER.warning(
                            "dropping spec '%s': unknown source_id '%s'",
                            spec.name,
                            spec.source_id,
                        )
                        continue
                    if not snippet_is_verbatim(spec.source_snippet, source_content):
                        LOGGER.warning(
                            "dropping spec '%s' of '%s': snippet is not verbatim",
                            spec.name,
                            product.name,
                        )
                        continue
                    kept_specs.append(spec)
                if not kept_specs:
                    LOGGER.info("dropping product '%s': no verified specs", product.name)
                    continue

                supplier_id = None
                supplier_name = product.supplier_name.strip()
                if supplier_name:
                    domain = _domain(product.supplier_url or product.canonical_url)
                    supplier_id = await connection.fetchval(
                        """INSERT INTO suppliers (name, domain, url)
                           VALUES ($1, $2, $3)
                           ON CONFLICT (name, domain) DO UPDATE SET url = EXCLUDED.url
                           RETURNING id""",
                        supplier_name,
                        domain,
                        product.supplier_url.strip(),
                    )

                content_hash = _content_hash(
                    product.canonical_url, product.name, product.model
                )
                product_id = await connection.fetchval(
                    """INSERT INTO products
                           (name, manufacturer, model, supplier_id, price, currency,
                            lead_time_days, canonical_url, source_id, confidence, content_hash)
                       VALUES ($1, $2, $3, $4, $5::float8, $6, $7, $8, $9, $10::float8, $11)
                       ON CONFLICT (content_hash) DO UPDATE
                       SET price = EXCLUDED.price,
                           currency = EXCLUDED.currency,
                           lead_time_days = EXCLUDED.lead_time_days,
                           supplier_id = EXCLUDED.supplier_id,
                           confidence = EXCLUDED.confidence
                       RETURNING id""",
                    product.name.strip(),
                    product.manufacturer.strip(),
                    product.model.strip(),
                    supplier_id,
                    product.price,
                    product.currency.strip() or None,
                    product.lead_time_days,
                    product.canonical_url.strip(),
                    uuid.UUID(product.source_id) if _is_uuid(product.source_id) else None,
                    product.confidence,
                    content_hash,
                )
                products_created += 1

                for spec in kept_specs:
                    await connection.execute(
                        """INSERT INTO product_specs
                               (product_id, name, value_text, value_num, unit, data_type,
                                source_id, source_snippet, confidence)
                           VALUES ($1, $2, $3, $4::float8, $5, $6, $7, $8, $9::float8)
                           ON CONFLICT (product_id, name) DO UPDATE
                           SET value_text = EXCLUDED.value_text,
                               value_num = EXCLUDED.value_num,
                               unit = EXCLUDED.unit,
                               data_type = EXCLUDED.data_type,
                               source_id = EXCLUDED.source_id,
                               source_snippet = EXCLUDED.source_snippet,
                               confidence = EXCLUDED.confidence""",
                        product_id,
                        spec.name.strip(),
                        spec.value_text.strip(),
                        spec.value_num,
                        spec.unit.strip(),
                        spec.data_type,
                        uuid.UUID(spec.source_id) if _is_uuid(spec.source_id) else None,
                        spec.source_snippet.strip(),
                        spec.confidence,
                    )
                    specs_created += 1

    return products_created, specs_created


def _is_uuid(value: str) -> bool:
    try:
        uuid.UUID(value)
        return True
    except (ValueError, AttributeError, TypeError):
        return False


def notice(
    envelope: common_pb2.JobEnvelope | None,
    component_id: str,
    products_created: int,
    success: bool,
    error_message: str = "",
) -> ai_pb2.ProductsExtractedNotice:
    return ai_pb2.ProductsExtractedNotice(
        envelope=envelope,
        component_id=component_id,
        products_created=products_created,
        success=success,
        error_message=error_message[:2000],
    )


__all__ = ["build_user_prompt", "extract", "notice", "persist"]
