"""OpenAI-compatible chat client that only returns validated JSON objects.

Both the primary and the fallback model are addressed through the same
``LLM_BASE_URL`` (any OpenAI-compatible gateway works). Failures are retried
once with a short backoff; after that the job fails with ``LlmError`` so the
worker records a real failure instead of fabricating data.
"""

from __future__ import annotations

import asyncio
import json
import logging

import httpx

from common.config import Config

LOGGER = logging.getLogger(__name__)

RETRIES = 2


class LlmError(RuntimeError):
    """The model call failed or returned unusable output."""


class LlmClient:
    def __init__(self, cfg: Config) -> None:
        self._cfg = cfg
        self._client = httpx.AsyncClient(
            base_url=cfg.llm_base_url,
            timeout=cfg.llm_timeout,
            headers={
                "Authorization": f"Bearer {cfg.require_llm()}",
                "Content-Type": "application/json",
            },
        )

    def model_name(self, escalate: bool = False) -> str:
        if escalate and self._cfg.llm_fallback_model:
            return self._cfg.llm_fallback_model
        return self._cfg.llm_model

    async def complete_json(
        self,
        *,
        system: str,
        user: str,
        escalate: bool = False,
        max_tokens: int | None = None,
    ) -> tuple[dict, int, int]:
        """Return ``(payload, prompt_tokens, completion_tokens)``."""
        payload = {
            "model": self.model_name(escalate),
            "temperature": self._cfg.llm_temperature,
            "max_tokens": max_tokens or self._cfg.llm_max_tokens,
            "response_format": {"type": "json_object"},
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user},
            ],
        }

        last_error: Exception | None = None
        for attempt in range(1, RETRIES + 1):
            try:
                response = await self._client.post("/chat/completions", json=payload)
                response.raise_for_status()
                data = response.json()
                content = data["choices"][0]["message"]["content"]
                parsed = json.loads(content)
                if not isinstance(parsed, dict):
                    raise LlmError("model returned a JSON value that is not an object")
                usage = data.get("usage") or {}
                tokens_in = int(usage.get("prompt_tokens") or 0)
                tokens_out = int(usage.get("completion_tokens") or 0)
                return parsed, tokens_in, tokens_out
            except (
                httpx.HTTPError,
                KeyError,
                IndexError,
                TypeError,
                ValueError,
                json.JSONDecodeError,
                LlmError,
            ) as error:
                last_error = error
                LOGGER.warning("LLM attempt %s/%s failed: %s", attempt, RETRIES, error)
                await asyncio.sleep(attempt)

        raise LlmError(f"LLM call failed: {last_error}")

    async def close(self) -> None:
        await self._client.aclose()
