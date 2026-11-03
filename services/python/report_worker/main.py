"""Report Worker entrypoint: ``jobs:reports`` -> XLSX/PDF/DOCX files.

Every generated file gets a ``reports`` row (READY with size and SHA-256) and a
``ReportReadyNotice`` is published so the orchestrator can finish the tender.
"""

from __future__ import annotations

import asyncio
import logging
import uuid

import asyncpg

from common.config import Config
from common.db import connect as connect_db
from common.progress import publish as publish_progress
from common.storage import sha256_hex, write_bytes
from common.streams import EVENTS_REPORTS, GROUP_REPORT_WORKERS, JOBS_REPORTS, Message, Streams
from common.worker import BusinessError, Worker
from generated.reports.v1 import report_pb2

from .builders import BUILDERS, ReportBuildError

LOGGER = logging.getLogger(__name__)


class ReportWorker(Worker):
    stream = JOBS_REPORTS
    group = GROUP_REPORT_WORKERS

    def __init__(self, streams: Streams, pool: asyncpg.Pool, cfg: Config, consumer: str) -> None:
        super().__init__(streams, consumer)
        self._pool = pool
        self._cfg = cfg

    def dedup_key(self, message: Message) -> str | None:
        job = self._decode(message)
        if job is None or job.envelope is None:
            return None
        return f"dedup:reports:{job.envelope.task_id}"

    @staticmethod
    def _decode(message: Message) -> report_pb2.GenerateReportJob | None:
        try:
            return report_pb2.GenerateReportJob.FromString(message.payload)
        except Exception as error:  # noqa: BLE001
            LOGGER.error("undecodable GenerateReportJob (%s): %s", message.id, error)
            return None

    async def handle(self, message: Message) -> None:
        job = self._decode(message)
        if job is None:
            raise BusinessError("undecodable report job")
        try:
            tender_id = uuid.UUID(job.tender_id)
            run_id = uuid.UUID(job.run_id)
        except ValueError as error:
            raise BusinessError(f"invalid ids in report job: {error}") from error

        formats = [fmt.lower() for fmt in job.formats] or ["xlsx", "docx", "pdf"]
        unknown = [fmt for fmt in formats if fmt not in BUILDERS]
        if unknown:
            raise BusinessError(f"unsupported report formats: {', '.join(unknown)}")

        await publish_progress(
            self.streams,
            event_type="report.started",
            tender_id=str(tender_id),
            run_id=str(run_id),
            stage="REPORT_GENERATION",
            progress=0.0,
            message=f"building {', '.join(formats)}",
        )

        tender, rows, details = await self._load(tender_id, run_id)
        if tender is None:
            raise BusinessError(f"tender {tender_id} does not exist")

        files = []
        try:
            for index, fmt in enumerate(formats):
                builder = BUILDERS[fmt]
                try:
                    data = builder(tender, rows, details)
                except ReportBuildError as error:
                    raise BusinessError(str(error)) from error
                key = f"{tender_id}/report-{uuid.uuid4().hex}.{fmt}"
                write_bytes(self._cfg, key, data)
                report_id = await self._insert_report(tender_id, run_id, fmt, key, data)
                files.append(
                    report_pb2.GeneratedFile(
                        format=fmt,
                        storage_key=key,
                        size_bytes=len(data),
                        sha256=sha256_hex(data),
                    )
                )
                await publish_progress(
                    self.streams,
                    event_type="report.file_ready",
                    tender_id=str(tender_id),
                    run_id=str(run_id),
                    stage="REPORT_GENERATION",
                    progress=(index + 1) / len(formats),
                    message=f"{fmt} ready ({len(data)} bytes)",
                    level="success",
                )
                LOGGER.info("report %s (%s) ready for tender %s", report_id, fmt, tender_id)
        except BusinessError as error:
            await self._mark_failed(tender_id, run_id, str(error))
            raise

        notice = report_pb2.ReportReadyNotice(
            envelope=job.envelope,
            tender_id=str(tender_id),
            run_id=str(run_id),
            files_created=len(files),
            success=True,
            error_message="",
        )
        await self.streams.xadd(EVENTS_REPORTS, notice.SerializeToString(), kind="report")

    async def _load(
        self, tender_id: uuid.UUID, run_id: uuid.UUID
    ) -> tuple[dict | None, list[dict], list[dict]]:
        async with self._pool.acquire() as connection:
            tender = await connection.fetchrow(
                "SELECT id, name, description, status FROM tenders WHERE id = $1", tender_id
            )
            rows = await connection.fetch(
                """SELECT c.position_no, c.name AS component_name,
                          c.quantity::float8 AS quantity, c.unit,
                          m.rank, m.verdict, m.eligible,
                          m.technical_score::float8 AS technical_score,
                          m.total_score::float8 AS total_score,
                          m.explanation,
                          p.name AS product_name, p.manufacturer, p.model,
                          p.price::float8 AS price, p.currency::text AS currency,
                          p.lead_time_days, p.canonical_url,
                          sup.name AS supplier_name, sup.url AS supplier_url
                   FROM components c
                   LEFT JOIN matches m
                          ON m.component_id = c.id AND m.run_id = $2
                   LEFT JOIN products p ON p.id = m.product_id
                   LEFT JOIN suppliers sup ON sup.id = p.supplier_id
                   WHERE c.tender_id = $1
                   ORDER BY c.position_no, m.rank NULLS LAST, m.total_score DESC NULLS LAST""",
                tender_id,
                run_id,
            )
            details = await connection.fetch(
                """SELECT c.position_no, d.requirement_name, d.status, d.expected,
                          d.actual, d.unit, d.explanation
                   FROM match_details d
                   JOIN matches m ON m.id = d.match_id
                   JOIN components c ON c.id = m.component_id
                   WHERE m.run_id = $2 AND c.tender_id = $1
                   ORDER BY c.position_no, d.match_id, d.requirement_name""",
                tender_id,
                run_id,
            )
        return (
            dict(tender) if tender is not None else None,
            [dict(row) for row in rows],
            [dict(detail) for detail in details],
        )

    async def _insert_report(
        self,
        tender_id: uuid.UUID,
        run_id: uuid.UUID,
        fmt: str,
        key: str,
        data: bytes,
    ) -> uuid.UUID:
        async with self._pool.acquire() as connection:
            report_id = await connection.fetchval(
                """INSERT INTO reports
                       (tender_id, run_id, format, storage_key, size_bytes, sha256,
                        status, ready_at)
                   VALUES ($1, $2, $3, $4, $5, $6, 'READY', now())
                   RETURNING id""",
                tender_id,
                run_id,
                fmt,
                key,
                len(data),
                sha256_hex(data),
            )
        return report_id

    async def _mark_failed(self, tender_id: uuid.UUID, run_id: uuid.UUID, message: str) -> None:
        async with self._pool.acquire() as connection:
            await connection.execute(
                """INSERT INTO reports (tender_id, run_id, format, storage_key, status, error_message)
                   VALUES ($1, $2, 'pdf', '', 'FAILED', $3)""",
                tender_id,
                run_id,
                message[:2000],
            )
        notice = report_pb2.ReportReadyNotice(
            tender_id=str(tender_id),
            run_id=str(run_id),
            files_created=0,
            success=False,
            error_message=message[:2000],
        )
        await self.streams.xadd(EVENTS_REPORTS, notice.SerializeToString(), kind="report")
        await publish_progress(
            self.streams,
            event_type="report.failed",
            tender_id=str(tender_id),
            run_id=str(run_id),
            stage="REPORT_GENERATION",
            level="error",
            message=message,
        )


async def main() -> None:
    cfg = Config.from_env("report-worker")
    logging.basicConfig(
        level=cfg.log_level.upper(),
        format="%(asctime)s %(levelname)s %(name)s %(message)s",
    )
    pool = await connect_db(cfg)
    streams = await Streams.connect(cfg.redis_url)
    workers = [
        ReportWorker(streams, pool, cfg, f"{cfg.consumer_identity}-{index}")
        for index in range(max(1, cfg.worker_concurrency))
    ]
    LOGGER.info("report worker starting with %s consumer(s)", len(workers))
    try:
        await asyncio.gather(*(worker.run() for worker in workers))
    finally:
        await streams.close()
        await pool.close()


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        logging.info("report worker stopped")
