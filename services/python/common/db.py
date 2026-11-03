"""PostgreSQL access shared by the Python workers."""

from __future__ import annotations

import asyncpg

from .config import Config


async def connect(cfg: Config) -> asyncpg.Pool:
    return await asyncpg.create_pool(
        dsn=cfg.database_url,
        min_size=1,
        max_size=max(1, cfg.database_max_connections),
        command_timeout=120,
        init=_init_connection,
    )


async def _init_connection(connection: asyncpg.Connection) -> None:
    # JSONB columns come back as JSON text; decode them in Python explicitly.
    await connection.set_type_codec(
        "jsonb",
        encoder=lambda value: value,
        decoder=lambda value: value,
        schema="pg_catalog",
        format="text",
    )
