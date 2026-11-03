"""``ProgressEvent`` construction and publication.

The event is serialized protobuf over the ``events:progress`` channel; the API
decodes it and forwards JSON to browser WebSockets. Publishing never fails a
business operation — progress is advisory, PostgreSQL is authoritative.
"""

from __future__ import annotations

import logging
import time
import uuid

from generated.common.v1 import common_pb2

from .streams import Streams

LOGGER = logging.getLogger(__name__)


async def publish(
    streams: Streams,
    *,
    event_type: str,
    tender_id: str,
    task_id: str = "",
    run_id: str = "",
    stage: str = "",
    progress: float = 0.0,
    message: str = "",
    level: str = "info",
) -> None:
    event = common_pb2.ProgressEvent(
        event_id=str(uuid.uuid4()),
        event_type=event_type,
        tender_id=tender_id,
        task_id=task_id,
        run_id=run_id,
        timestamp_unix_ms=int(time.time() * 1000),
        stage=stage,
        progress=max(0.0, min(1.0, progress)),
        message=message,
        level=level,
    )
    try:
        await streams.publish_progress(event.SerializeToString())
    except Exception as error:  # noqa: BLE001 - progress is best effort
        LOGGER.warning("progress publish failed: %s", error)
