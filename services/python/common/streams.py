"""Redis Streams topology and consumer helpers (mirror of the Rust module).

Every job and event payload is a serialized protobuf message; entries carry a
``payload`` field and an optional ``kind`` discriminator. Delivery is
at-least-once, so handlers deduplicate by logical task id.
"""

from __future__ import annotations

import logging
from dataclasses import dataclass

import redis.asyncio as redis
from redis.exceptions import ResponseError

LOGGER = logging.getLogger(__name__)

# ---- job streams ----
JOBS_DOCUMENTS = "jobs:documents"
JOBS_AI_REQUIREMENTS = "jobs:ai.requirements"
JOBS_AI_PRODUCTS = "jobs:ai.products"
JOBS_REPORTS = "jobs:reports"

# ---- event streams ----
EVENTS_DOCUMENTS = "events:documents"
EVENTS_AI = "events:ai"
EVENTS_REPORTS = "events:reports"

# ---- pub/sub ----
PROGRESS_CHANNEL = "events:progress"

# ---- consumer groups ----
GROUP_DOC_WORKERS = "doc-workers"
GROUP_AI_WORKERS = "ai-workers"
GROUP_REPORT_WORKERS = "report-workers"

FIELD_PAYLOAD = b"payload"
FIELD_KIND = b"kind"


@dataclass(frozen=True)
class Message:
    id: str
    kind: str
    payload: bytes


def _decode_fields(fields: dict[bytes, bytes]) -> Message | None:
    payload = fields.get(FIELD_PAYLOAD)
    if payload is None:
        return None
    kind = fields.get(FIELD_KIND) or b""
    return Message(id="", kind=kind.decode("utf-8", "replace"), payload=payload)


def _parse_entry(entry) -> Message | None:
    entry_id, fields = entry
    message = _decode_fields(fields)
    if message is None:
        LOGGER.warning("stream entry %s has no payload field; skipping", entry_id)
        return None
    return Message(
        id=entry_id.decode("utf-8", "replace")
        if isinstance(entry_id, bytes)
        else str(entry_id),
        kind=message.kind,
        payload=message.payload,
    )


class Streams:
    """Thin async wrapper around a Redis connection."""

    def __init__(self, client: redis.Redis) -> None:
        self._client = client

    @classmethod
    async def connect(cls, redis_url: str) -> "Streams":
        client = redis.from_url(redis_url, decode_responses=False)
        await client.ping()
        return cls(client)

    async def close(self) -> None:
        await self._client.aclose()

    async def ping(self) -> None:
        await self._client.ping()

    async def ensure_group(self, stream: str, group: str) -> None:
        try:
            await self._client.xgroup_create(stream, group, id="0", mkstream=True)
        except ResponseError as error:
            if "BUSYGROUP" not in str(error):
                raise

    async def read(
        self,
        stream: str,
        group: str,
        consumer: str,
        count: int = 4,
        block_ms: int = 5_000,
    ) -> list[Message]:
        response = await self._client.xreadgroup(
            group, consumer, {stream: ">"}, count=count, block=block_ms
        )
        if not response:
            return []
        messages: list[Message] = []
        for _stream_name, entries in response:
            for entry in entries:
                parsed = _parse_entry(entry)
                if parsed is not None:
                    messages.append(parsed)
        return messages

    async def autoclaim(
        self,
        stream: str,
        group: str,
        consumer: str,
        min_idle_ms: int = 600_000,
        count: int = 8,
    ) -> list[Message]:
        response = await self._client.xautoclaim(
            stream, group, consumer, min_idle_time=min_idle_ms, start_id="0", count=count
        )
        # redis-py returns (next_id, entries, deleted_ids)
        entries = response[1] if len(response) > 1 else []
        messages: list[Message] = []
        for entry in entries:
            if not entry:
                continue
            parsed = _parse_entry(entry)
            if parsed is not None:
                messages.append(parsed)
        return messages

    async def ack(self, stream: str, group: str, ids: list[str]) -> None:
        if ids:
            await self._client.xack(stream, group, *ids)

    async def xadd(self, stream: str, payload: bytes, kind: str = "") -> str:
        entry_id = await self._client.xadd(
            stream, {FIELD_PAYLOAD: payload, FIELD_KIND: kind.encode("utf-8")}
        )
        return entry_id.decode("utf-8", "replace")

    async def publish_progress(self, payload: bytes) -> int:
        return int(await self._client.publish(PROGRESS_CHANNEL, payload))

    async def set_nx(self, key: str, ttl_seconds: int) -> bool:
        result = await self._client.set(key, "1", nx=True, ex=ttl_seconds)
        return bool(result)

    async def release(self, key: str) -> None:
        await self._client.delete(key)
