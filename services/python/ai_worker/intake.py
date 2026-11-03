"""Free-form intake: turn an operator's plain sentence into tender positions.

The operator writes what they need the way they would say it out loud
("нужно 200 метров кабеля ВВГнг 3х2,5 и десяток автоматов на 16 ампер"), the
model returns the same request as structured positions with the
characteristics that were actually stated.

Nothing is invented: a specification is kept only when its value appears in the
operator's text (numbers are matched as written), so a request never grows
requirements the buyer did not ask for. Everything the model returns is
validated by [`IntakeResult`] before it reaches the database.
"""

from __future__ import annotations

import json
import logging
import re
import uuid

import asyncpg

from .llm import LlmClient
from .schemas import IntakeResult

LOGGER = logging.getLogger(__name__)

MAX_TEXT_CHARS = 4_000

SYSTEM_PROMPT = """You are a procurement analyst for a Russian supply company.
The operator describes a purchase in plain language; you turn it into tender positions.

Return ONLY a JSON object:
{"tender_name": "short title of the purchase, <= 80 chars",
 "summary": "one sentence describing what was understood",
 "positions": [{"name": "product name as the buyer would search for it",
                "quantity": 10, "unit": "шт", "category": "short category",
                "requirements": [{"name": "Напряжение", "value_text": "660",
                                  "value_num": 660, "unit": "В",
                                  "data_type": "NUMBER", "mandatory": false}]}]}

Rules:
1. One position per distinct product the operator asked for.
2. quantity is the number the operator wrote (default 1); unit is the unit they
   used ("м", "шт", "кг", "уп"); keep Russian units, never convert.
3. requirements contain ONLY characteristics stated in the text: voltage,
   current, cross-section, material, standard, dimensions, protection class.
   Never invent a value, never add "typical" specs.
4. Each requirement: data_type is one of STRING, NUMBER, RANGE, BOOLEAN, ENUM,
   STANDARD, MATERIAL, MODEL, DIMENSION. Put the numeric value into value_num
   when the requirement is numeric.
5. Names are canonical and short ("Кабель ВВГнг-LS 3х2,5", not a sentence).
6. If the text contains no products at all, return {"positions": []}.
7. Answer in Russian, product names in Russian exactly as written."""


def build_user_prompt(text: str) -> str:
    return (
        "REQUEST FROM THE OPERATOR (verbatim):\n"
        f"{text[:MAX_TEXT_CHARS]}\n\n"
        "Turn this request into structured tender positions as JSON."
    )


async def extract(
    llm: LlmClient, text: str
) -> tuple[IntakeResult, int, int]:
    payload, tokens_in, tokens_out = await llm.complete_json(
        system=SYSTEM_PROMPT, user=build_user_prompt(text)
    )
    return IntakeResult.model_validate(payload), tokens_in, tokens_out


async def record_usage(
    pool: asyncpg.Pool,
    *,
    model: str,
    request_id: uuid.UUID,
    tokens_in: int,
    tokens_out: int,
) -> None:
    """Intake happens before a tender exists, so ``tender_id`` stays NULL."""
    async with pool.acquire() as connection:
        await connection.execute(
            """INSERT INTO api_usage (service, sku, task_id, quantity, tokens_in, tokens_out)
               VALUES ('llm', $1, $2, 1, $3, $4)""",
            model,
            f"intake:{request_id}",
            tokens_in,
            tokens_out,
        )


# ---------------------------------------------------------------------------
# Agent trace: the four stages shown in the UI, persisted after each change.
# ---------------------------------------------------------------------------

STAGES: tuple[tuple[str, str], ...] = (
    ("read", "Читаю запрос"),
    ("structure", "Собираю позиции и характеристики"),
    ("check", "Проверяю количества, единицы и значения"),
    ("save", "Сохраняю структурированный запрос"),
)

# Progress implied by a stage being the current one.
STAGE_PROGRESS = (0.05, 0.25, 0.8, 0.95)


class Trace:
    """Persists the step list and progress of one intake request."""

    def __init__(self, pool: asyncpg.Pool, request_id: uuid.UUID) -> None:
        self._pool = pool
        self._request_id = request_id
        self._steps = [
            {"key": key, "title": title, "status": "pending", "detail": ""}
            for key, title in STAGES
        ]

    def _render(self) -> str:
        return json.dumps(self._steps, ensure_ascii=False)

    async def _write(self, status: str, progress: float, error: str | None = None) -> None:
        async with self._pool.acquire() as connection:
            await connection.execute(
                """UPDATE intake_requests
                      SET status = $2, progress = $3, steps = $4::jsonb,
                          error_message = $5, updated_at = now()
                    WHERE id = $1""",
                self._request_id,
                status,
                progress,
                self._render(),
                error,
            )

    async def begin(self, stage: int, detail: str = "") -> None:
        for index, step in enumerate(self._steps):
            if index < stage and step["status"] in ("pending", "running"):
                step["status"] = "done"
            elif index == stage:
                step["status"] = "running"
                step["detail"] = detail
            elif step["status"] == "running":
                step["status"] = "done"
        await self._write("RUNNING", STAGE_PROGRESS[min(stage, len(STAGES) - 1)])

    async def finish(self, detail: str) -> None:
        for step in self._steps:
            if step["status"] != "error":
                step["status"] = "done"
        self._steps[-1]["detail"] = detail
        await self._write("COMPLETED", 1.0)

    async def fail(self, message: str) -> None:
        for step in self._steps:
            if step["status"] == "running":
                step["status"] = "error"
                step["detail"] = message
                break
        await self._write("FAILED", 1.0, message[:2000])


# ---------------------------------------------------------------------------
# Provenance: keep only the values the operator actually wrote.
# ---------------------------------------------------------------------------

_NUMBER = re.compile(r"\d+(?:[.,]\d+)?")


def _numbers_in(text: str) -> set[str]:
    return {match.replace(",", ".") for match in _NUMBER.findall(text)}


def drop_unstated(positions: list, request_text: str) -> tuple[list, int]:
    """Remove requirements whose value never appears in the request.

    A requirement is kept when its number (if any) is written in the text or,
    for text values, when the value words occur in the text. This is the same
    anti-hallucination rule the document extractor applies with snippets.
    """
    text = request_text.replace(",", ".")
    known_numbers = _numbers_in(request_text)
    haystack = re.sub(r"\s+", " ", request_text.replace("ё", "е")).lower()
    kept_positions = []
    dropped = 0

    for position in positions:
        kept = []
        for requirement in position.requirements:
            if requirement.value_num is not None:
                candidate = f"{requirement.value_num:g}"
                if candidate in known_numbers or candidate.replace(".", ",") in text:
                    kept.append(requirement)
                else:
                    dropped += 1
                    LOGGER.info(
                        "dropping requirement '%s=%s': value is not in the request",
                        requirement.name,
                        requirement.value_text,
                    )
                continue
            value = normalize(requirement.value_text)
            if not value or value in haystack:
                kept.append(requirement)
            else:
                dropped += 1
                LOGGER.info(
                    "dropping requirement '%s=%s': value is not in the request",
                    requirement.name,
                    requirement.value_text,
                )
        position.requirements = kept
        kept_positions.append(position)
    return kept_positions, dropped


def normalize(value: str) -> str:
    return re.sub(r"\s+", " ", (value or "").replace("ё", "е")).strip().lower()


async def save(
    pool: asyncpg.Pool, request_id: uuid.UUID, result: IntakeResult
) -> None:
    async with pool.acquire() as connection:
        await connection.execute(
            "UPDATE intake_requests SET result = $2::jsonb WHERE id = $1",
            request_id,
            result.model_dump_json(),
        )
