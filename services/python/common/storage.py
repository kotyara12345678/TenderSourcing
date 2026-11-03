"""Local file storage access for workers (mirror of the Rust storage rules)."""

from __future__ import annotations

import hashlib
from pathlib import Path

from .config import Config


class StorageError(RuntimeError):
    """A storage key was invalid or the file is missing."""


def base_path(cfg: Config) -> Path:
    return Path(cfg.storage_path)


def resolve(cfg: Config, key: str) -> Path:
    candidate = Path(key)
    if candidate.is_absolute() or any(part == ".." for part in candidate.parts):
        raise StorageError(f"invalid storage key '{key}'")
    return base_path(cfg) / candidate


def read_bytes(cfg: Config, key: str) -> bytes:
    path = resolve(cfg, key)
    try:
        return path.read_bytes()
    except FileNotFoundError as error:
        raise StorageError(f"file '{key}' does not exist") from error


def write_bytes(cfg: Config, key: str, data: bytes) -> Path:
    path = resolve(cfg, key)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)
    return path


def sha256_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()
