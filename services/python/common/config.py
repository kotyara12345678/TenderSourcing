"""Validated environment configuration shared by every Python worker.

Variable names and defaults match ``.env.example`` exactly. Problems are
collected and reported together so an operator can fix the environment in a
single pass.
"""

from __future__ import annotations

import os
import socket
from dataclasses import dataclass


class ConfigError(SystemExit):
    """Raised when the environment is invalid; the message lists every problem."""


def _raw(name: str) -> str | None:
    value = os.environ.get(name, "").strip()
    return value or None


def _int(name: str, default: int, errors: list[str]) -> int:
    value = _raw(name)
    if value is None:
        return default
    try:
        return int(value)
    except ValueError:
        errors.append(f"{name} must be an integer, got '{value}'")
        return default


def _float(name: str, default: float, errors: list[str]) -> float:
    value = _raw(name)
    if value is None:
        return default
    try:
        return float(value)
    except ValueError:
        errors.append(f"{name} must be a number, got '{value}'")
        return default


def _bool(name: str, default: bool, errors: list[str]) -> bool:
    value = _raw(name)
    if value is None:
        return default
    lowered = value.lower()
    if lowered in {"1", "true", "yes", "on"}:
        return True
    if lowered in {"0", "false", "no", "off"}:
        return False
    errors.append(f"{name} must be a boolean, got '{value}'")
    return default


@dataclass(frozen=True)
class Config:
    app_env: str
    log_level: str
    database_url: str
    database_max_connections: int
    redis_url: str
    storage_provider: str
    storage_path: str
    llm_api_key: str | None
    llm_base_url: str
    llm_model: str
    llm_fallback_model: str
    llm_timeout: float
    llm_max_tokens: int
    llm_temperature: float
    ocr_provider: str
    ocr_api_key: str | None
    ocr_base_url: str | None
    max_file_size: int
    max_llm_tokens_per_tender: int
    worker_concurrency: int
    ai_concurrency: int
    document_concurrency: int
    consumer_identity: str

    @classmethod
    def from_env(cls, service: str) -> "Config":
        errors: list[str] = []
        for name in ("DATABASE_URL", "REDIS_URL"):
            if _raw(name) is None:
                errors.append(f"{name} is required")

        hostname = socket.gethostname() or "local"
        identity = f"{hostname}-{service}-{os.getpid()}"

        config = cls(
            app_env=_raw("APP_ENV") or "development",
            log_level=(_raw("LOG_LEVEL") or "info").lower(),
            database_url=_raw("DATABASE_URL") or "",
            database_max_connections=_int("DATABASE_MAX_CONNECTIONS", 5, errors),
            redis_url=_raw("REDIS_URL") or "",
            storage_provider=_raw("STORAGE_PROVIDER") or "local",
            storage_path=_raw("STORAGE_PATH") or "/data/storage",
            llm_api_key=_raw("LLM_API_KEY"),
            llm_base_url=(_raw("LLM_BASE_URL") or "https://api.openai.com/v1").rstrip("/"),
            llm_model=_raw("LLM_MODEL") or "gpt-4o-mini",
            llm_fallback_model=_raw("LLM_FALLBACK_MODEL") or "",
            llm_timeout=_float("LLM_TIMEOUT", 120.0, errors),
            llm_max_tokens=_int("LLM_MAX_TOKENS", 8000, errors),
            llm_temperature=_float("LLM_TEMPERATURE", 0.0, errors),
            ocr_provider=_raw("OCR_PROVIDER") or "none",
            ocr_api_key=_raw("OCR_API_KEY"),
            ocr_base_url=_raw("OCR_BASE_URL"),
            max_file_size=_int("MAX_FILE_SIZE", 26_214_400, errors),
            max_llm_tokens_per_tender=_int("MAX_LLM_TOKENS_PER_TENDER", 2_000_000, errors),
            worker_concurrency=_int("WORKER_CONCURRENCY", 4, errors),
            ai_concurrency=_int("AI_CONCURRENCY", 2, errors),
            document_concurrency=_int("DOCUMENT_CONCURRENCY", 2, errors),
            consumer_identity=identity,
        )

        if config.storage_provider not in {"local", "s3"}:
            errors.append("STORAGE_PROVIDER must be 'local' or 's3'")
        if config.storage_provider == "s3":
            errors.append(
                "STORAGE_PROVIDER=s3 is not implemented in v1; use STORAGE_PROVIDER=local"
            )
        if config.ocr_provider not in {"none", "api"}:
            errors.append("OCR_PROVIDER must be 'none' or 'api'")
        if config.ocr_provider == "api" and not config.ocr_base_url:
            errors.append("OCR_BASE_URL is required when OCR_PROVIDER=api")
        if errors:
            raise ConfigError("configuration error: " + "; ".join(errors))
        return config

    def require_llm(self) -> str:
        """Return the API key or fail loudly — AI workers cannot run without it."""
        if not self.llm_api_key:
            raise ConfigError(
                "configuration error: LLM_API_KEY is required for the AI worker"
            )
        return self.llm_api_key
