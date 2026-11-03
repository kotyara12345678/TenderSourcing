"""Content extraction for uploaded tender documents.

Every page keeps structure: plain text in reading order, tables as rows and
positioned text blocks with normalized bounding boxes. Native text carries
``confidence = 1.0``; OCR text is marked ``ocr_used`` with a lower confidence.

Known assumptions (documented in ``docs/PIPELINE.md``):

* DOCX has no page concept — paragraphs are grouped into virtual pages of
  ``DOCX_BLOCKS_PER_PAGE`` blocks so provenance stays addressable;
* XLSX maps one sheet to one page;
* RTF is reduced to plain text with a conservative control-word strip;
* OCR requires ``OCR_PROVIDER=api`` — otherwise pages without a text layer are
  stored as image pages with empty text instead of invented content.
"""

from __future__ import annotations

import asyncio
import base64
import io
import logging
import re
from dataclasses import dataclass, field
from typing import Any

import httpx

LOGGER = logging.getLogger(__name__)

DOCX_BLOCKS_PER_PAGE = 40
PDF_TEXT_FLAG = "text"


@dataclass
class ExtractedPage:
    page_number: int
    content_type: str
    text: str
    blocks: list[dict[str, Any]] = field(default_factory=list)
    tables: list[dict[str, Any]] = field(default_factory=list)
    ocr_used: bool = False


class OcrClient:
    """Generic OCR HTTP client (``OCR_PROVIDER=api``)."""

    def __init__(self, base_url: str, api_key: str | None) -> None:
        self._url = base_url.rstrip("/")
        headers = {"X-API-Key": api_key} if api_key else {}
        self._client = httpx.AsyncClient(timeout=120.0, headers=headers)

    async def extract(self, image_png: bytes) -> str:
        response = await self._client.post(
            self._url,
            json={"image_base64": base64.b64encode(image_png).decode("ascii")},
        )
        response.raise_for_status()
        payload = response.json()
        text = payload.get("text")
        if not isinstance(text, str):
            raise RuntimeError("OCR provider returned no 'text' field")
        return text

    async def close(self) -> None:
        await self._client.aclose()


def build_ocr(provider: str, base_url: str | None, api_key: str | None) -> OcrClient | None:
    if provider == "api" and base_url:
        return OcrClient(base_url, api_key)
    return None


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _normalize_blocks(blocks: list[dict[str, Any]]) -> list[dict[str, Any]]:
    return blocks


def _text_from_blocks(blocks: list[dict[str, Any]]) -> str:
    return "\n".join(block["text"] for block in blocks if block.get("text"))


# ---------------------------------------------------------------------------
# PDF
# ---------------------------------------------------------------------------


async def extract_pdf(
    data: bytes, run_ocr: bool, ocr: OcrClient | None
) -> tuple[list[ExtractedPage], bool]:
    import fitz  # PyMuPDF

    document = fitz.open(stream=data, filetype="pdf")
    pages: list[ExtractedPage] = []
    any_ocr = False
    try:
        for index, page in enumerate(document):
            page_number = index + 1
            text = page.get_text(PDF_TEXT_FLAG) or ""
            tables: list[dict[str, Any]] = []
            table_text_parts: list[str] = []

            try:
                finder = page.find_tables()
                for table_index, table in enumerate(finder.tables):
                    raw_rows = table.extract() or []
                    rows = [
                        [(cell or "").strip() for cell in row]
                        for row in raw_rows
                    ]
                    if not rows:
                        continue
                    tables.append({"index": table_index, "rows": rows})
                    table_text_parts.append(
                        "\n".join("\t".join(row) for row in rows)
                    )
            except Exception as error:  # noqa: BLE001 - table detection is best effort
                LOGGER.debug("table detection failed on page %s: %s", page_number, error)

            width = page.rect.width or 1.0
            height = page.rect.height or 1.0
            blocks: list[dict[str, Any]] = []
            try:
                block_dict = page.get_text("dict")
                for block in block_dict.get("blocks", []):
                    if block.get("type") != 0:
                        continue
                    block_text = "\n".join(
                        span.get("text", "")
                        for line in block.get("lines", [])
                        for span in line.get("spans", [])
                    ).strip()
                    if not block_text:
                        continue
                    bbox = block.get("bbox", (0.0, 0.0, 0.0, 0.0))
                    blocks.append(
                        {
                            "text": block_text,
                            "bbox": {
                                "x": float(bbox[0]) / width,
                                "y": float(bbox[1]) / height,
                                "width": (float(bbox[2]) - float(bbox[0])) / width,
                                "height": (float(bbox[3]) - float(bbox[1])) / height,
                            },
                            "confidence": 1.0,
                            "kind": "text",
                        }
                    )
            except Exception as error:  # noqa: BLE001
                LOGGER.debug("block extraction failed on page %s: %s", page_number, error)

            ocr_used = False
            if not text.strip() and not blocks and run_ocr and ocr is not None:
                try:
                    pixmap = page.get_pixmap(dpi=200)
                    ocr_text = await ocr.extract(pixmap.tobytes("png"))
                    if ocr_text.strip():
                        text = ocr_text
                        ocr_used = True
                        any_ocr = True
                        blocks = [
                            {
                                "text": line,
                                "bbox": {
                                    "x": 0.0,
                                    "y": 0.0,
                                    "width": 1.0,
                                    "height": 0.0,
                                },
                                "confidence": 0.6,
                                "kind": "ocr",
                            }
                            for line in ocr_text.splitlines()
                            if line.strip()
                        ]
                except Exception as error:  # noqa: BLE001
                    LOGGER.warning("OCR failed on page %s: %s", page_number, error)

            if table_text_parts:
                text = (text + "\n" + "\n".join(table_text_parts)).strip()

            content_type = "text"
            if tables and text:
                content_type = "mixed"
            elif tables:
                content_type = "table"
            elif not text.strip():
                content_type = "image"

            pages.append(
                ExtractedPage(
                    page_number=page_number,
                    content_type=content_type,
                    text=text.strip(),
                    blocks=blocks,
                    tables=tables,
                    ocr_used=ocr_used,
                )
            )
    finally:
        document.close()

    if not pages:
        pages.append(
            ExtractedPage(page_number=1, content_type="text", text="", blocks=[], tables=[])
        )
    return pages, any_ocr


# ---------------------------------------------------------------------------
# DOCX
# ---------------------------------------------------------------------------


def extract_docx(data: bytes) -> tuple[list[ExtractedPage], bool]:
    from docx import Document

    document = Document(io.BytesIO(data))
    entries: list[tuple[str, str]] = []
    for paragraph in document.paragraphs:
        text = paragraph.text.strip()
        if text:
            entries.append(("text", text))
    for table_index, table in enumerate(document.tables):
        rows = [
            [cell.text.strip() for cell in row.cells]
            for row in table.rows
        ]
        if not rows:
            continue
        entries.append(
            (
                "table",
                "\n".join("\t".join(row) for row in rows),
            )
        )
        entries.append(("table_index", str(table_index)))

    if not entries:
        return [
            ExtractedPage(page_number=1, content_type="text", text="", blocks=[], tables=[])
        ], False

    pages: list[ExtractedPage] = []
    for offset in range(0, len(entries), DOCX_BLOCKS_PER_PAGE):
        chunk = entries[offset : offset + DOCX_BLOCKS_PER_PAGE]
        texts = [text for kind, text in chunk if kind in {"text", "table"}]
        tables = [
            {"index": int(text), "rows": []}
            for kind, text in chunk
            if kind == "table_index"
        ]
        pages.append(
            ExtractedPage(
                page_number=len(pages) + 1,
                content_type="mixed" if tables else "text",
                text="\n".join(texts).strip(),
                blocks=[
                    {
                        "text": text,
                        "bbox": {"x": 0.0, "y": 0.0, "width": 1.0, "height": 0.0},
                        "confidence": 1.0,
                        "kind": "text",
                    }
                    for text in texts
                ],
                tables=tables,
            )
        )
    return pages, False


# ---------------------------------------------------------------------------
# XLSX
# ---------------------------------------------------------------------------


def extract_xlsx(data: bytes) -> tuple[list[ExtractedPage], bool]:
    from openpyxl import load_workbook

    workbook = load_workbook(io.BytesIO(data), data_only=True, read_only=True)
    pages: list[ExtractedPage] = []
    try:
        for worksheet in workbook.worksheets:
            rows: list[list[str]] = []
            for row in worksheet.iter_rows(values_only=True):
                cells = ["" if value is None else str(value) for value in row]
                if any(cell.strip() for cell in cells):
                    rows.append(cells)
            text = "\n".join("\t".join(row) for row in rows)
            pages.append(
                ExtractedPage(
                    page_number=len(pages) + 1,
                    content_type="table" if rows else "text",
                    text=text.strip(),
                    blocks=[],
                    tables=[{"index": 0, "rows": rows}] if rows else [],
                )
            )
    finally:
        workbook.close()

    if not pages:
        pages.append(
            ExtractedPage(page_number=1, content_type="text", text="", blocks=[], tables=[])
        )
    return pages, False


# ---------------------------------------------------------------------------
# TXT / RTF
# ---------------------------------------------------------------------------

_RTF_CONTROL = re.compile(r"\\[a-zA-Z]+-?\d* ?|[{}]")


def extract_text(data: bytes, is_rtf: bool = False) -> tuple[list[ExtractedPage], bool]:
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError:
        text = data.decode("cp1251", errors="replace")
    if is_rtf:
        text = _RTF_CONTROL.sub(" ", text)
        text = re.sub(r"[ \t]+", " ", text)
    text = text.strip()
    return (
        [
            ExtractedPage(
                page_number=1,
                content_type="text",
                text=text,
                blocks=(
                    [
                        {
                            "text": text,
                            "bbox": {"x": 0.0, "y": 0.0, "width": 1.0, "height": 0.0},
                            "confidence": 1.0,
                            "kind": "text",
                        }
                    ]
                    if text
                    else []
                ),
            )
        ],
        False,
    )


async def extract_document(
    filename: str,
    mime_type: str,
    data: bytes,
    run_ocr: bool,
    ocr: OcrClient | None,
) -> tuple[list[ExtractedPage], bool]:
    extension = filename.rsplit(".", 1)[-1].lower() if "." in filename else ""
    if extension == "pdf" or mime_type == "application/pdf":
        return await extract_pdf(data, run_ocr, ocr)
    if extension == "docx":
        return await asyncio.to_thread(extract_docx, data)
    if extension == "xlsx":
        return await asyncio.to_thread(extract_xlsx, data)
    if extension == "rtf":
        return await asyncio.to_thread(extract_text, data, True)
    return await asyncio.to_thread(extract_text, data)


__all__ = [
    "ExtractedPage",
    "OcrClient",
    "build_ocr",
    "extract_document",
    "extract_docx",
    "extract_pdf",
    "extract_text",
    "extract_xlsx",
]
