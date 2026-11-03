"""Report builders.

The report always answers three questions per position: which offer is best,
why it won (score breakdown + verdict) and which requirement checks support
that verdict. Unknown data is printed as "—", never as a fabricated value.
"""

from __future__ import annotations

import io
import logging
import os

LOGGER = logging.getLogger(__name__)

UNKNOWN = "—"

BEST_HEADERS = [
    "№",
    "Позиция",
    "Кол-во",
    "Ед.",
    "Продукт",
    "Производитель",
    "Модель",
    "Цена",
    "Валюта",
    "Срок, дн.",
    "Поставщик",
    "Ссылка",
    "Балл",
    "Вердикт",
]

ALL_HEADERS = ["№", "Ранг", "Продукт", "Модель", "Цена", "Балл", "Вердикт", "Пояснение", "Ссылка"]
DETAIL_HEADERS = ["№", "Требование", "Статус", "Ожидалось", "Фактически", "Ед.", "Пояснение"]


class ReportBuildError(RuntimeError):
    """A report could not be produced in the requested format."""


def _fmt(value) -> str:
    if value is None:
        return UNKNOWN
    if isinstance(value, float):
        return f"{value:.4f}".rstrip("0").rstrip(".") if value % 1 else str(int(value))
    text = str(value).strip()
    return text or UNKNOWN


def _best_rows(rows: list[dict]) -> list[list[str]]:
    best: list[list[str]] = []
    for row in rows:
        if row["rank"] != 1:
            continue
        best.append(
            [
                str(row["position_no"]),
                str(row["component_name"]),
                _fmt(row["quantity"]),
                _fmt(row["unit"]),
                _fmt(row["product_name"]),
                _fmt(row["manufacturer"]),
                _fmt(row["model"]),
                _fmt(row["price"]),
                _fmt(row["currency"]),
                _fmt(row["lead_time_days"]),
                _fmt(row["supplier_name"]),
                _fmt(row["canonical_url"]),
                _fmt(row["total_score"]),
                _fmt(row["verdict"]),
            ]
        )
    return best


def _all_rows(rows: list[dict]) -> list[list[str]]:
    return [
        [
            str(row["position_no"]),
            _fmt(row["rank"]),
            _fmt(row["product_name"]),
            _fmt(row["model"]),
            _fmt(row["price"]),
            _fmt(row["total_score"]),
            _fmt(row["verdict"]),
            _fmt(row["explanation"]),
            _fmt(row["canonical_url"]),
        ]
        for row in rows
    ]


def _detail_rows(details: list[dict]) -> list[list[str]]:
    return [
        [
            str(detail["position_no"]),
            _fmt(detail["requirement_name"]),
            _fmt(detail["status"]),
            _fmt(detail["expected"]),
            _fmt(detail["actual"]),
            _fmt(detail["unit"]),
            _fmt(detail["explanation"]),
        ]
        for detail in details
    ]


# ---------------------------------------------------------------------------
# XLSX
# ---------------------------------------------------------------------------


def build_xlsx(tender: dict, rows: list[dict], details: list[dict]) -> bytes:
    from openpyxl import Workbook
    from openpyxl.styles import Alignment, Font

    workbook = Workbook()
    sheet = workbook.active
    sheet.title = "Позиции"
    sheet.append(BEST_HEADERS)
    for cell in sheet[1]:
        cell.font = Font(bold=True)
    for row in _best_rows(rows):
        sheet.append(row)
    sheet.freeze_panes = "A2"

    all_sheet = workbook.create_sheet("Все совпадения")
    all_sheet.append(ALL_HEADERS)
    for cell in all_sheet[1]:
        cell.font = Font(bold=True)
    for row in _all_rows(rows):
        all_sheet.append(row)
    all_sheet.freeze_panes = "A2"

    detail_sheet = workbook.create_sheet("Проверки требований")
    detail_sheet.append(DETAIL_HEADERS)
    for cell in detail_sheet[1]:
        cell.font = Font(bold=True)
    for row in _detail_rows(details):
        detail_sheet.append(row)
    detail_sheet.freeze_panes = "A2"

    for worksheet in workbook.worksheets:
        for row in worksheet.iter_rows():
            for cell in row:
                cell.alignment = Alignment(wrap_text=False, vertical="top")

    buffer = io.BytesIO()
    workbook.save(buffer)
    LOGGER.info("xlsx report built for tender '%s'", tender.get("name", ""))
    return buffer.getvalue()


# ---------------------------------------------------------------------------
# PDF
# ---------------------------------------------------------------------------

DEJAVU_REGULAR = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"
DEJAVU_BOLD = "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"


def _register_fonts() -> None:
    from reportlab.pdfbase import pdfmetrics
    from reportlab.pdfbase.ttfonts import TTFont

    if not (os.path.exists(DEJAVU_REGULAR) and os.path.exists(DEJAVU_BOLD)):
        raise ReportBuildError(
            "DejaVu fonts are required for Cyrillic PDF output but were not found at "
            f"{DEJAVU_REGULAR}. Install fonts-dejavu-core in the worker image "
            "(deploy/docker/Dockerfile.worker)."
        )
    pdfmetrics.registerFont(TTFont("DejaVu", DEJAVU_REGULAR))
    pdfmetrics.registerFont(TTFont("DejaVu-Bold", DEJAVU_BOLD))


def build_pdf(tender: dict, rows: list[dict], details: list[dict]) -> bytes:
    from reportlab.lib import colors
    from reportlab.lib.pagesizes import A4, landscape
    from reportlab.lib.styles import ParagraphStyle, getSampleStyleSheet
    from reportlab.lib.units import mm
    from reportlab.platypus import (
        PageBreak,
        Paragraph,
        SimpleDocTemplate,
        Spacer,
        Table,
        TableStyle,
    )

    _register_fonts()

    styles = getSampleStyleSheet()
    title_style = ParagraphStyle(
        "TitleDejaVu",
        parent=styles["Title"],
        fontName="DejaVu-Bold",
        fontSize=16,
        leading=20,
    )
    heading_style = ParagraphStyle(
        "HeadingDejaVu",
        parent=styles["Heading2"],
        fontName="DejaVu-Bold",
        fontSize=12,
        leading=16,
        spaceBefore=8,
    )
    body_style = ParagraphStyle(
        "BodyDejaVu",
        parent=styles["BodyText"],
        fontName="DejaVu",
        fontSize=9,
        leading=12,
    )

    buffer = io.BytesIO()
    document = SimpleDocTemplate(
        buffer,
        pagesize=landscape(A4),
        leftMargin=12 * mm,
        rightMargin=12 * mm,
        topMargin=12 * mm,
        bottomMargin=12 * mm,
        title=f"Отчёт по тендеру: {tender.get('name', '')}",
        author="TenderSourcing",
    )

    story: list = []
    story.append(Paragraph("Отчёт по подбору поставщиков", title_style))
    story.append(Paragraph(f"Тендер: {tender.get('name', UNKNOWN)}", body_style))
    if tender.get("description"):
        story.append(Paragraph(str(tender["description"]), body_style))
    story.append(Spacer(1, 6 * mm))

    def table(rows_data: list[list[str]], widths: list[float]) -> Table:
        data = [[Paragraph(cell, body_style) for cell in row] for row in rows_data]
        result = Table(data, colWidths=widths, repeatRows=1)
        result.setStyle(
            TableStyle(
                [
                    ("GRID", (0, 0), (-1, -1), 0.25, colors.grey),
                    ("BACKGROUND", (0, 0), (-1, 0), colors.whitesmoke),
                    ("VALIGN", (0, 0), (-1, -1), "TOP"),
                ]
            )
        )
        return result

    story.append(Paragraph("Лучшее предложение по каждой позиции", heading_style))
    best = _best_rows(rows)
    if best:
        widths = [10 * mm, 55 * mm, 15 * mm, 12 * mm, 50 * mm, 30 * mm, 25 * mm, 20 * mm, 14 * mm, 16 * mm, 30 * mm, 45 * mm, 14 * mm, 22 * mm]
        story.append(table([BEST_HEADERS] + best, widths))
    else:
        story.append(Paragraph("Совпадения не найдены.", body_style))

    story.append(PageBreak())
    story.append(Paragraph("Все совпадения", heading_style))
    all_rows = _all_rows(rows)
    if all_rows:
        widths = [10 * mm, 12 * mm, 60 * mm, 30 * mm, 20 * mm, 14 * mm, 22 * mm, 80 * mm, 40 * mm]
        story.append(table([ALL_HEADERS] + all_rows, widths))

    story.append(PageBreak())
    story.append(Paragraph("Проверка требований", heading_style))
    detail_rows = _detail_rows(details)
    if detail_rows:
        widths = [10 * mm, 50 * mm, 20 * mm, 45 * mm, 45 * mm, 12 * mm, 90 * mm]
        story.append(table([DETAIL_HEADERS] + detail_rows, widths))
    else:
        story.append(Paragraph("Проверки требований отсутствуют.", body_style))

    document.build(story)
    LOGGER.info("pdf report built for tender '%s'", tender.get("name", ""))
    return buffer.getvalue()


# ---------------------------------------------------------------------------
# DOCX
# ---------------------------------------------------------------------------


def build_docx(tender: dict, rows: list[dict], details: list[dict]) -> bytes:
    from docx import Document
    from docx.shared import Pt

    document = Document()
    style = document.styles["Normal"]
    style.font.name = "DejaVu Sans"
    style.font.size = Pt(9)

    document.add_heading("Отчёт по подбору поставщиков", level=0)
    document.add_paragraph(f"Тендер: {tender.get('name', UNKNOWN)}")
    if tender.get("description"):
        document.add_paragraph(str(tender["description"]))

    def add_table(headers: list[str], rows_data: list[list[str]]) -> None:
        table = document.add_table(rows=1, cols=len(headers))
        table.style = "Table Grid"
        for index, header in enumerate(headers):
            table.rows[0].cells[index].text = header
        for row in rows_data:
            cells = table.add_row().cells
            for index, value in enumerate(row):
                if index < len(cells):
                    cells[index].text = value

    document.add_heading("Лучшее предложение по каждой позиции", level=1)
    best = _best_rows(rows)
    if best:
        add_table(BEST_HEADERS, best)
    else:
        document.add_paragraph("Совпадения не найдены.")

    document.add_heading("Все совпадения", level=1)
    all_rows = _all_rows(rows)
    if all_rows:
        add_table(ALL_HEADERS, all_rows)

    document.add_heading("Проверка требований", level=1)
    detail_rows = _detail_rows(details)
    if detail_rows:
        add_table(DETAIL_HEADERS, detail_rows)
    else:
        document.add_paragraph("Проверки требований отсутствуют.")

    buffer = io.BytesIO()
    document.save(buffer)
    LOGGER.info("docx report built for tender '%s'", tender.get("name", ""))
    return buffer.getvalue()


BUILDERS = {
    "xlsx": build_xlsx,
    "pdf": build_pdf,
    "docx": build_docx,
}

__all__ = ["BUILDERS", "ReportBuildError", "build_docx", "build_pdf", "build_xlsx"]
