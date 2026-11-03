"""Shared consumer loop for all Python workers.

At-least-once semantics:

* a logical task is guarded by a Redis ``SET NX`` key derived from the payload
  (``dedup_key``); a duplicate delivery is acknowledged and skipped;
* ``BusinessError`` marks the job as failed (the handler persists the failure
  and publishes a notice), then the message is acknowledged;
* any other exception is treated as infrastructure trouble: the dedup key is
  released and the message stays in the PEL so it is redelivered (or claimed
  by another replica via ``XAUTOCLAIM``);
* a message that keeps failing is dropped only after ``MAX_ATTEMPTS``
  attempts, always with an ERROR log.
"""

from __future__ import annotations

import abc
import asyncio
import logging

from .streams import Message, Streams

LOGGER = logging.getLogger(__name__)

MAX_ATTEMPTS = 5
DEDUP_TTL_SECONDS = 6 * 3600
AUTOCLAIM_IDLE_MS = 600_000


class BusinessError(RuntimeError):
    """The job cannot succeed; the handler recorded the failure."""


class Worker(abc.ABC):
    """Base consumer: subclass and implement ``handle``/``dedup_key``."""

    stream: str
    group: str
    consumer_name: str

    def __init__(self, streams: Streams, consumer: str) -> None:
        self.streams = streams
        self.consumer = consumer

    # ------------------------------------------------------------------
    # Subclass API
    # ------------------------------------------------------------------

    @abc.abstractmethod
    def dedup_key(self, message: Message) -> str | None:
        """Return the idempotency key, or None for undecodable messages."""

    @abc.abstractmethod
    async def handle(self, message: Message) -> None:
        """Process one message. Raise ``BusinessError`` for terminal failures."""

    # ------------------------------------------------------------------
    # Loop
    # ------------------------------------------------------------------

    async def run(self) -> None:
        await self.streams.ensure_group(self.stream, self.group)
        LOGGER.info(
            "consumer started stream=%s group=%s consumer=%s",
            self.stream,
            self.group,
            self.consumer,
        )
        attempts: dict[str, int] = {}
        while True:
            batch: list[Message] = []
            try:
                batch.extend(
                    await self.streams.autoclaim(
                        self.stream, self.group, self.consumer, AUTOCLAIM_IDLE_MS, 8
                    )
                )
            except Exception as error:  # noqa: BLE001
                LOGGER.warning("autoclaim failed: %s", error)

            try:
                batch.extend(await self.streams.read(self.stream, self.group, self.consumer))
            except asyncio.CancelledError:
                raise
            except Exception as error:  # noqa: BLE001
                LOGGER.error("stream read failed: %s; backing off", error)
                await asyncio.sleep(5)
                continue

            for message in batch:
                await self._process_one(message, attempts)

    async def _process_one(self, message: Message, attempts: dict[str, int]) -> None:
        key = self.dedup_key(message)
        if key is not None:
            try:
                acquired = await self.streams.set_nx(key, DEDUP_TTL_SECONDS)
            except Exception as error:  # noqa: BLE001
                LOGGER.error("dedup guard failed for %s: %s", message.id, error)
                return
            if not acquired:
                LOGGER.info("duplicate delivery of %s; acknowledging", message.id)
                await self._ack(message)
                return

        try:
            await self.handle(message)
        except BusinessError as error:
            LOGGER.warning("job %s failed permanently: %s", message.id, error)
            await self._ack(message)
            attempts.pop(message.id, None)
            return
        except asyncio.CancelledError:
            raise
        except Exception as error:  # noqa: BLE001
            LOGGER.error("job %s failed: %s", message.id, error)
            if key is not None:
                try:
                    await self.streams.release(key)
                except Exception as release_error:  # noqa: BLE001
                    LOGGER.warning("could not release dedup key: %s", release_error)
            count = attempts.get(message.id, 0) + 1
            attempts[message.id] = count
            if count >= MAX_ATTEMPTS:
                LOGGER.error(
                    "dropping job %s after %s attempts (stream %s)",
                    message.id,
                    count,
                    self.stream,
                )
                await self._ack(message)
                attempts.pop(message.id, None)
            return

        await self._ack(message)
        attempts.pop(message.id, None)

    async def _ack(self, message: Message) -> None:
        try:
            await self.streams.ack(self.stream, self.group, [message.id])
        except Exception as error:  # noqa: BLE001
            LOGGER.warning("ack failed for %s: %s", message.id, error)
