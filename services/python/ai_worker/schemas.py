"""Pydantic models mirroring ``proto/ai/v1/ai.proto`` plus provenance helpers.

Anti-hallucination rule: a value is only kept when its ``source_snippet`` is a
verbatim (whitespace-normalized) substring of the content the model was given.
Anything else is dropped, never silently "fixed".
"""

from __future__ import annotations

import re

from pydantic import BaseModel, Field, field_validator

DATA_TYPES = {
    "STRING",
    "NUMBER",
    "RANGE",
    "BOOLEAN",
    "ENUM",
    "STANDARD",
    "MATERIAL",
    "MODEL",
    "DIMENSION",
}


def normalize(text: str) -> str:
    """Collapse whitespace, lowercase, unify ё/е — used for verbatim checks."""
    return re.sub(r"\s+", " ", (text or "").replace("ё", "е")).strip().lower()


def snippet_is_verbatim(snippet: str, content: str) -> bool:
    snippet_norm = normalize(snippet)
    if len(snippet_norm) < 2:
        return False
    return snippet_norm in normalize(content)


class RequirementOut(BaseModel):
    name: str = Field(min_length=1, max_length=500)
    value_text: str = ""
    value_num: float | None = None
    unit: str = ""
    data_type: str = "STRING"
    mandatory: bool = False
    range_min: float | None = None
    range_max: float | None = None
    tolerance: float | None = None
    allowed_values: list[str] = Field(default_factory=list)
    confidence: float = 0.5
    source_snippet: str = ""

    @field_validator("data_type")
    @classmethod
    def _data_type(cls, value: str) -> str:
        normalized = (value or "STRING").strip().upper()
        return normalized if normalized in DATA_TYPES else "STRING"

    @field_validator("confidence")
    @classmethod
    def _confidence(cls, value: float) -> float:
        return max(0.0, min(1.0, float(value)))

    @field_validator("tolerance")
    @classmethod
    def _tolerance(cls, value: float | None) -> float | None:
        if value is None:
            return None
        return max(0.0, min(1.0, float(value)))

    @field_validator("allowed_values")
    @classmethod
    def _allowed(cls, values: list[str]) -> list[str]:
        return [str(item).strip() for item in values if str(item).strip()]


class ComponentOut(BaseModel):
    name: str = Field(min_length=1, max_length=500)
    quantity: float = 1.0
    unit: str = "pcs"
    category: str = ""
    search_hints: dict = Field(default_factory=dict)
    requirements: list[RequirementOut] = Field(default_factory=list)

    @field_validator("quantity")
    @classmethod
    def _quantity(cls, value: float) -> float:
        quantity = float(value)
        return quantity if quantity > 0 else 1.0

    @field_validator("category")
    @classmethod
    def _category(cls, value: str) -> str:
        return (value or "").strip()[:200]


class RequirementsExtraction(BaseModel):
    components: list[ComponentOut] = Field(default_factory=list)


class SpecOut(BaseModel):
    name: str = Field(min_length=1, max_length=500)
    value_text: str = ""
    value_num: float | None = None
    unit: str = ""
    data_type: str = "STRING"
    source_id: str = ""
    source_snippet: str = ""
    confidence: float = 0.5

    @field_validator("data_type")
    @classmethod
    def _data_type(cls, value: str) -> str:
        normalized = (value or "STRING").strip().upper()
        return normalized if normalized in DATA_TYPES else "STRING"

    @field_validator("confidence")
    @classmethod
    def _confidence(cls, value: float) -> float:
        return max(0.0, min(1.0, float(value)))


class ProductOut(BaseModel):
    name: str = Field(min_length=1, max_length=500)
    manufacturer: str = ""
    model: str = ""
    supplier_name: str = ""
    supplier_url: str = ""
    price: float | None = None
    currency: str = ""
    lead_time_days: int | None = None
    canonical_url: str = ""
    source_id: str = ""
    specs: list[SpecOut] = Field(default_factory=list)
    confidence: float = 0.5

    @field_validator("price")
    @classmethod
    def _price(cls, value: float | None) -> float | None:
        if value is None:
            return None
        price = float(value)
        return price if price >= 0 else None

    @field_validator("lead_time_days")
    @classmethod
    def _lead_time(cls, value: int | None) -> int | None:
        if value is None:
            return None
        days = int(value)
        return days if days >= 0 else None

    @field_validator("confidence")
    @classmethod
    def _confidence(cls, value: float) -> float:
        return max(0.0, min(1.0, float(value)))


class ProductsExtraction(BaseModel):
    products: list[ProductOut] = Field(default_factory=list)
