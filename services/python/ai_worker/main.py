"""AI Worker entrypoint.

Two consumers run in one process:

* ``jobs:ai.requirements`` — extracts positions/requirements from a document;
* ``jobs:ai.products`` — extracts product offers and specs from fetched pages;
* ``jobs:ai.intake`` — turns a free-form purchasing request into positions.

Both publish notices on ``events:ai`` (``kind`` = requirements | products) so
the orchestrator can advance the tender FSM.
"""

from __future__ import annotations

import asyncio
import logging
import uuid

import asyncpg

from common.config import Config
from common.db import connect as connect_db
from common.progress import publish as publish_progress
from common.streams import (
    EVENTS_AI,
    GROUP_AI_WORKERS,
    JOBS_AI_INTAKE,
    JOBS_AI_PRODUCTS,
    JOBS_AI_REQUIREMENTS,
    Message,
    Streams,
)
from common.worker import BusinessError, Worker
from generated.ai.v1 import ai_pb2

from . import intake as intake_module
from . import products as products_module
from . import requirements as requirements_module
from .llm import LlmClient, LlmError

LOGGER = logging.getLogger(__name__)


class RequirementsWorker(Worker):
    stream = JOBS_AI_REQUIREMENTS
    group = GROUP_AI_WORKERS

    def __init__(
        self, streams: Streams, pool: asyncpg.Pool, cfg: Config, llm: LlmClient, consumer: str
    ) -> None:
        super().__init__(streams, consumer)
        self._pool = pool
        self._cfg = cfg
        self._llm = llm

    def dedup_key(self, message: Message) -> str | None:
        job = self._decode(message)
        return f"dedup:ai.requirements:{job.document_id}" if job else None

    @staticmethod
    def _decode(message: Message) -> ai_pb2.ExtractRequirementsJob | None:
        try:
            return ai_pb2.ExtractRequirementsJob.FromString(message.payload)
        except Exception as error:  # noqa: BLE001
            LOGGER.error("undecodable ExtractRequirementsJob (%s): %s", message.id, error)
            return None

    async def handle(self, message: Message) -> None:
        job = self._decode(message)
        if job is None:
            raise BusinessError("undecodable requirements job")
        envelope = job.envelope
        if envelope is None:
            raise BusinessError("requirements job without an envelope")
        try:
            document_id = uuid.UUID(job.document_id)
            tender_id = uuid.UUID(envelope.tender_id)
        except ValueError as error:
            raise BusinessError(f"invalid ids in requirements job: {error}") from error

        await publish_progress(
            self.streams,
            event_type="ai.requirements.started",
            tender_id=str(tender_id),
            task_id=str(document_id),
            stage="REQUIREMENT_EXTRACTION",
            progress=0.0,
            message="extracting tender positions",
        )

        async with self._pool.acquire() as connection:
            await connection.execute(
                "UPDATE documents SET ai_status = 'RUNNING' WHERE id = $1", document_id
            )
            rows = await connection.fetch(
                "SELECT page_number, content FROM document_pages "
                "WHERE document_id = $1 ORDER BY page_number",
                document_id,
            )

        if not rows:
            await self._fail(document_id, job, str(tender_id), "document has no extracted pages")
            raise BusinessError("document has no extracted pages")

        document_text = "\n\n".join(
            f"[page {row['page_number']}]\n{row['content']}" for row in rows
        )

        if await requirements_module.llm_budget_exhausted(self._pool, self._cfg, tender_id):
            await self._fail(
                document_id,
                job,
                str(tender_id),
                "tender LLM token budget is exhausted",
            )
            raise BusinessError("tender LLM token budget is exhausted")

        try:
            extraction, tokens_in, tokens_out = await requirements_module.extract(
                self._llm, document_text, job.escalate
            )
        except LlmError as error:
            await self._fail(document_id, job, str(tender_id), str(error))
            raise BusinessError(str(error)) from error

        await requirements_module.record_usage(
            self._pool,
            model=self._llm.model_name(job.escalate),
            tender_id=tender_id,
            task_id=str(document_id),
            tokens_in=tokens_in,
            tokens_out=tokens_out,
        )

        components_created, requirements_created = await requirements_module.persist(
            self._pool,
            tender_id=tender_id,
            document_id=document_id,
            extraction=extraction,
            document_text=document_text,
            max_components=max(1, self._cfg.worker_concurrency * 100),
        )

        async with self._pool.acquire() as connection:
            await connection.execute(
                "UPDATE documents SET ai_status = 'DONE', error_message = NULL WHERE id = $1",
                document_id,
            )

        notice = requirements_module.notice(
            envelope,
            str(document_id),
            components_created,
            requirements_created,
            True,
        )
        await self.streams.xadd(EVENTS_AI, notice.SerializeToString(), kind="requirements")
        await publish_progress(
            self.streams,
            event_type="ai.requirements.completed",
            tender_id=str(tender_id),
            task_id=str(document_id),
            stage="REQUIREMENT_EXTRACTION",
            progress=1.0,
            message=f"{components_created} position(s), {requirements_created} requirement(s)",
            level="success",
        )
        LOGGER.info(
            "document %s: %s position(s), %s requirement(s)",
            document_id,
            components_created,
            requirements_created,
        )

    async def _fail(
        self,
        document_id: uuid.UUID,
        job: ai_pb2.ExtractRequirementsJob,
        tender_id: str,
        message: str,
    ) -> None:
        async with self._pool.acquire() as connection:
            await connection.execute(
                "UPDATE documents SET ai_status = 'FAILED', error_message = $2 WHERE id = $1",
                document_id,
                message[:2000],
            )
        notice = requirements_module.notice(
            job.envelope, str(document_id), 0, 0, False, message
        )
        await self.streams.xadd(EVENTS_AI, notice.SerializeToString(), kind="requirements")
        await publish_progress(
            self.streams,
            event_type="ai.requirements.failed",
            tender_id=tender_id,
            task_id=str(document_id),
            stage="REQUIREMENT_EXTRACTION",
            level="error",
            message=message,
        )


class ProductsWorker(Worker):
    stream = JOBS_AI_PRODUCTS
    group = GROUP_AI_WORKERS

    def __init__(
        self, streams: Streams, pool: asyncpg.Pool, cfg: Config, llm: LlmClient, consumer: str
    ) -> None:
        super().__init__(streams, consumer)
        self._pool = pool
        self._cfg = cfg
        self._llm = llm

    def dedup_key(self, message: Message) -> str | None:
        job = self._decode(message)
        if job is None or job.envelope is None:
            return None
        return f"dedup:ai.products:{job.envelope.task_id}"

    @staticmethod
    def _decode(message: Message) -> ai_pb2.ExtractProductsJob | None:
        try:
            return ai_pb2.ExtractProductsJob.FromString(message.payload)
        except Exception as error:  # noqa: BLE001
            LOGGER.error("undecodable ExtractProductsJob (%s): %s", message.id, error)
            return None

    async def handle(self, message: Message) -> None:
        job = self._decode(message)
        if job is None:
            raise BusinessError("undecodable products job")
        envelope = job.envelope
        if envelope is None:
            raise BusinessError("products job without an envelope")
        try:
            component_id = uuid.UUID(job.component_id)
            tender_id = uuid.UUID(envelope.tender_id)
            task_id = uuid.UUID(envelope.task_id)
        except ValueError as error:
            raise BusinessError(f"invalid ids in products job: {error}") from error

        async with self._pool.acquire() as connection:
            component = await connection.fetchrow(
                "SELECT name FROM components WHERE id = $1", component_id
            )
            requirement_rows = await connection.fetch(
                "SELECT name, value_text, unit FROM requirements WHERE component_id = $1 ORDER BY name",
                component_id,
            )
            source_rows = await connection.fetch(
                """SELECT s.id, s.url, s.title, s.content
                   FROM search_results r
                   JOIN sources s ON s.id = r.source_id
                   WHERE r.task_id = $1 AND r.source_id IS NOT NULL
                   ORDER BY r.relevance_score DESC NULLS LAST
                   LIMIT $2""",
                task_id,
                products_module.MAX_SOURCES,
            )

        if component is None:
            raise BusinessError(f"component {component_id} does not exist")
        if not source_rows:
            notice = products_module.notice(envelope, str(component_id), 0, True)
            await self.streams.xadd(EVENTS_AI, notice.SerializeToString(), kind="products")
            LOGGER.info("component %s has no fetched sources; nothing to extract", component_id)
            return

        sources: dict[str, str] = {
            str(row["id"]): row["content"] or "" for row in source_rows
        }
        source_tuples = [
            (str(row["id"]), row["url"], row["title"], row["content"] or "")
            for row in source_rows
        ]
        requirement_labels = [
            f"{row['name']}: {row['value_text']} {row['unit']}".strip()
            for row in requirement_rows
        ]

        if await requirements_module.llm_budget_exhausted(self._pool, self._cfg, tender_id):
            message_text = "tender LLM token budget is exhausted"
            notice = products_module.notice(envelope, str(component_id), 0, False, message_text)
            await self.streams.xadd(EVENTS_AI, notice.SerializeToString(), kind="products")
            raise BusinessError(message_text)

        try:
            extraction, tokens_in, tokens_out = await products_module.extract(
                self._llm,
                component["name"],
                requirement_labels,
                source_tuples,
                job.escalate,
            )
        except LlmError as error:
            notice = products_module.notice(envelope, str(component_id), 0, False, str(error))
            await self.streams.xadd(EVENTS_AI, notice.SerializeToString(), kind="products")
            await publish_progress(
                self.streams,
                event_type="ai.products.failed",
                tender_id=str(tender_id),
                task_id=str(task_id),
                stage="PRODUCT_EXTRACTION",
                level="error",
                message=str(error),
            )
            raise BusinessError(str(error)) from error

        await requirements_module.record_usage(
            self._pool,
            model=self._llm.model_name(job.escalate),
            tender_id=tender_id,
            task_id=str(task_id),
            tokens_in=tokens_in,
            tokens_out=tokens_out,
        )

        products_created, specs_created = await products_module.persist(
            self._pool,
            extraction=extraction,
            sources=sources,
            max_products=max(1, self._cfg.worker_concurrency * 10),
        )

        notice = products_module.notice(
            envelope, str(component_id), products_created, True
        )
        await self.streams.xadd(EVENTS_AI, notice.SerializeToString(), kind="products")
        await publish_progress(
            self.streams,
            event_type="ai.products.completed",
            tender_id=str(tender_id),
            task_id=str(task_id),
            stage="PRODUCT_EXTRACTION",
            progress=1.0,
            message=f"{products_created} product(s), {specs_created} spec(s)",
            level="success",
        )
        LOGGER.info(
            "component %s: %s product(s), %s spec(s)",
            component_id,
            products_created,
            specs_created,
        )


class IntakeWorker(Worker):
    """Free-form request -> structured positions (no tender exists yet)."""

    stream = JOBS_AI_INTAKE
    group = GROUP_AI_WORKERS

    def __init__(
        self, streams: Streams, pool: asyncpg.Pool, cfg: Config, llm: LlmClient, consumer: str
    ) -> None:
        super().__init__(streams, consumer)
        self._pool = pool
        self._cfg = cfg
        self._llm = llm

    @staticmethod
    def _decode(message: Message) -> ai_pb2.ParseIntakeJob | None:
        try:
            return ai_pb2.ParseIntakeJob.FromString(message.payload)
        except Exception as error:  # noqa: BLE001
            LOGGER.error("undecodable ParseIntakeJob (%s): %s", message.id, error)
            return None

    def dedup_key(self, message: Message) -> str | None:
        job = self._decode(message)
        return f"dedup:ai.intake:{job.request_id}" if job and job.request_id else None

    async def handle(self, message: Message) -> None:
        job = self._decode(message)
        if job is None:
            raise BusinessError("undecodable intake job")
        try:
            request_id = uuid.UUID(job.request_id)
        except ValueError as error:
            raise BusinessError(f"invalid intake request id: {error}") from error

        text = job.text.strip()
        if len(text) < 8:
            trace = intake_module.Trace(self._pool, request_id)
            await trace.begin(0)
            await trace.fail("запрос слишком короткий: опишите, что и сколько нужно")
            raise BusinessError("intake request is too short")

        trace = intake_module.Trace(self._pool, request_id)
        await trace.begin(0, f"{len(text)} символов")

        await trace.begin(1)
        try:
            result, tokens_in, tokens_out = await intake_module.extract(self._llm, text)
        except LlmError as error:
            await trace.fail(str(error))
            raise BusinessError(str(error)) from error

        await intake_module.record_usage(
            self._pool,
            model=self._llm.model_name(False),
            request_id=request_id,
            tokens_in=tokens_in,
            tokens_out=tokens_out,
        )

        await trace.begin(2)
        positions, dropped = intake_module.drop_unstated(result.positions, text)
        positions = [position for position in positions if position.name.strip()]
        result.positions = positions

        if not result.positions:
            message_text = "не удалось найти товарные позиции в запросе"
            await trace.fail(message_text)
            raise BusinessError(message_text)

        await trace.begin(3)
        await intake_module.save(self._pool, request_id, result)
        detail = f"{len(result.positions)} поз."
        if dropped:
            detail += f", отброшено неподтверждённых характеристик: {dropped}"
        await trace.finish(detail)
        LOGGER.info(
            "intake %s: %s position(s), %s requirement(s) dropped",
            request_id,
            len(result.positions),
            dropped,
        )


async def main() -> None:
    cfg = Config.from_env("ai-worker")
    logging.basicConfig(
        level=cfg.log_level.upper(),
        format="%(asctime)s %(levelname)s %(name)s %(message)s",
    )
    pool = await connect_db(cfg)
    streams = await Streams.connect(cfg.redis_url)
    llm = LlmClient(cfg)

    concurrency = max(1, cfg.ai_concurrency)
    workers: list[Worker] = []
    for index in range(concurrency):
        workers.append(
            RequirementsWorker(streams, pool, cfg, llm, f"{cfg.consumer_identity}-req-{index}")
        )
        workers.append(
            ProductsWorker(streams, pool, cfg, llm, f"{cfg.consumer_identity}-prod-{index}")
        )
        workers.append(
            IntakeWorker(streams, pool, cfg, llm, f"{cfg.consumer_identity}-intake-{index}")
        )

    LOGGER.info("ai worker starting with %s consumer(s)", len(workers))
    try:
        await asyncio.gather(*(worker.run() for worker in workers))
    finally:
        await llm.close()
        await streams.close()
        await pool.close()


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        logging.info("ai worker stopped")
