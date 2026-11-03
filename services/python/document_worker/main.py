"""Document Worker entrypoint: ``jobs:documents`` -> ``document_pages``.

Flow per job:

1. mark the document ``PROCESSING``;
2. read the uploaded file from storage and extract pages;
3. upsert ``document_pages`` (idempotent on ``(document_id, page_number)``);
4. mark the document ``EXTRACTED`` with page count and OCR flag;
5. publish a ``DocumentExtractedNotice`` on ``events:documents``.

Failures mark the document ``FAILED`` with the reason and still publish a
notice (``success = false``) so the pipeline never stalls silently.
"""

from __future__ import annotations

import asyncio
import json
import logging
import uuid
from typing import Any

import asyncpg

from common.config import Config
from common.db import connect as connect_db
from common.progress import publish as publish_progress
from common.storage import StorageError, read_bytes
from common.streams import EVENTS_DOCUMENTS, GROUP_DOC_WORKERS, JOBS_DOCUMENTS, Message, Streams
from common.worker import BusinessError, Worker
from generated.common.v1 import common_pb2
from generated.documents.v1 import document_pb2

from .extract import build_ocr, extract_document

LOGGER = logging.getLogger(__name__)


class DocumentWorker(Worker):
    stream = JOBS_DOCUMENTS
    group = GROUP_DOC_WORKERS

    def __init__(
        self,
        streams: Streams,
        pool: asyncpg.Pool,
        cfg: Config,
        consumer: str,
    ) -> None:
        super().__init__(streams, consumer)
        self._pool = pool
        self._cfg = cfg
        self._ocr = build_ocr(cfg.ocr_provider, cfg.ocr_base_url, cfg.ocr_api_key)

    # ------------------------------------------------------------------

    def dedup_key(self, message: Message) -> str | None:
        job = self._decode(message)
        return f"dedup:documents:{job.document_id}" if job is not None else None

    @staticmethod
    def _decode(message: Message) -> document_pb2.ExtractDocumentJob | None:
        try:
            return document_pb2.ExtractDocumentJob.FromString(message.payload)
        except Exception as error:  # noqa: BLE001
            LOGGER.error("undecodable ExtractDocumentJob (%s): %s", message.id, error)
            return None

    async def handle(self, message: Message) -> None:
        job = self._decode(message)
        if job is None:
            raise BusinessError("undecodable job")
        try:
            document_id = uuid.UUID(job.document_id)
        except ValueError as error:
            raise BusinessError(f"invalid document id '{job.document_id}'") from error

        envelope = job.envelope
        tender_id = envelope.tender_id if envelope else ""

        await self._mark_processing(document_id)

        try:
            data = read_bytes(self._cfg, job.storage_key)
        except StorageError as error:
            await self._fail(document_id, job, tender_id, str(error))
            raise BusinessError(str(error)) from error

        try:
            pages, ocr_used = await extract_document(
                job.original_filename,
                job.mime_type,
                data,
                job.run_ocr,
                self._ocr,
            )
        except Exception as error:  # noqa: BLE001 - parser failure is terminal
            message_text = f"cannot parse '{job.original_filename}': {error}"
            await self._fail(document_id, job, tender_id, message_text)
            raise BusinessError(message_text) from error

        await self._persist_pages(document_id, pages)

        total_chars = sum(len(page.text) for page in pages)
        async with self._pool.acquire() as connection:
            await connection.execute(
                """UPDATE documents
                   SET status = 'EXTRACTED', page_count = $2, ocr_used = $3,
                       processed_at = now(), error_message = NULL
                   WHERE id = $1""",
                document_id,
                len(pages),
                ocr_used,
            )

        notice = document_pb2.DocumentExtractedNotice(
            envelope=envelope,
            document_id=str(document_id),
            page_count=len(pages),
            total_chars=total_chars,
            ocr_used=ocr_used,
            success=True,
            error_message="",
        )
        await self.streams.xadd(EVENTS_DOCUMENTS, notice.SerializeToString())
        await publish_progress(
            self.streams,
            event_type="document.extracted",
            tender_id=tender_id,
            task_id=str(document_id),
            stage="DOCUMENT_PROCESSING",
            progress=1.0,
            message=f"{job.original_filename}: {len(pages)} page(s), {total_chars} chars",
            level="success",
        )
        LOGGER.info(
            "document %s extracted: %s page(s), %s chars (ocr=%s)",
            document_id,
            len(pages),
            total_chars,
            ocr_used,
        )

    # ------------------------------------------------------------------

    async def _mark_processing(self, document_id: uuid.UUID) -> None:
        async with self._pool.acquire() as connection:
            await connection.execute(
                "UPDATE documents SET status = 'PROCESSING', error_message = NULL WHERE id = $1",
                document_id,
            )

    async def _persist_pages(self, document_id: uuid.UUID, pages: list[Any]) -> None:
        async with self._pool.acquire() as connection:
            async with connection.transaction():
                for page in pages:
                    await connection.execute(
                        """INSERT INTO document_pages
                               (document_id, page_number, content_type, content, blocks, tables, ocr_used)
                           VALUES ($1, $2, $3, $4, $5::jsonb, $6::jsonb, $7)
                           ON CONFLICT (document_id, page_number) DO UPDATE
                           SET content_type = EXCLUDED.content_type,
                               content = EXCLUDED.content,
                               blocks = EXCLUDED.blocks,
                               tables = EXCLUDED.tables,
                               ocr_used = EXCLUDED.ocr_used""",
                        document_id,
                        page.page_number,
                        page.content_type,
                        page.text,
                        json.dumps(page.blocks, ensure_ascii=False),
                        json.dumps(page.tables, ensure_ascii=False),
                        page.ocr_used,
                    )

    async def _fail(
        self,
        document_id: uuid.UUID,
        job: document_pb2.ExtractDocumentJob,
        tender_id: str,
        message: str,
    ) -> None:
        async with self._pool.acquire() as connection:
            await connection.execute(
                """UPDATE documents
                   SET status = 'FAILED', error_message = $2, processed_at = now()
                   WHERE id = $1""",
                document_id,
                message[:2000],
            )
        notice = document_pb2.DocumentExtractedNotice(
            envelope=job.envelope,
            document_id=str(document_id),
            page_count=0,
            total_chars=0,
            ocr_used=False,
            success=False,
            error_message=message[:2000],
        )
        await self.streams.xadd(EVENTS_DOCUMENTS, notice.SerializeToString())
        await publish_progress(
            self.streams,
            event_type="document.failed",
            tender_id=tender_id,
            task_id=str(document_id),
            stage="DOCUMENT_PROCESSING",
            progress=0.0,
            message=f"{job.original_filename}: {message}",
            level="error",
        )


async def main() -> None:
    cfg = Config.from_env("document-worker")
    logging.basicConfig(
        level=cfg.log_level.upper(),
        format="%(asctime)s %(levelname)s %(name)s %(message)s",
    )
    pool = await connect_db(cfg)
    streams = await Streams.connect(cfg.redis_url)
    workers = [
        DocumentWorker(streams, pool, cfg, f"{cfg.consumer_identity}-{index}")
        for index in range(max(1, cfg.document_concurrency))
    ]
    LOGGER.info("document worker starting with %s consumer(s)", len(workers))
    try:
        await asyncio.gather(*(worker.run() for worker in workers))
    finally:
        await streams.close()
        await pool.close()


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        logging.info("document worker stopped")
