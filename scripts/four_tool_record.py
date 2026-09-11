"""Tool-neutral per-target record for the four-tool benchmark.

The record deliberately separates a tool's self-reported route from the
common post-hoc audit. Adapters may retain their complete raw output beside
this row; only stable, comparable fields belong here.
"""

from __future__ import annotations

import json
from dataclasses import asdict, dataclass, field
from typing import Any

SCHEMA_VERSION = "renkin-four-tool-row/1"
TOOLS = frozenset({"renkin", "aizynthfinder", "syntheseus", "synplanner"})
RUN_STATUSES = frozenset(
    {"completed", "timeout", "crashed", "setup_error", "parse_error", "not_measured"}
)


class FourToolRecordError(ValueError):
    """Raised when an adapter violates the common row contract."""


@dataclass
class FourToolRecord:
    target_id: str
    target_smiles: str
    sample_rank: int
    tool: str
    arm_id: str
    run_status: str

    # Tool-native signal. It must not be used as the common audit result.
    route_found: bool | None = None
    tool_reported_route_count: int | None = None

    # Common, post-hoc signal. None means the route was not available or was
    # not evaluable; False is a completed evaluation that failed.
    common_route_parseable: bool | None = None
    strict_route_to_shared_stock: bool | None = None
    common_audit_status: str | None = None

    total_elapsed_ms: float | None = None
    planning_elapsed_ms: float | None = None
    cold_start_elapsed_ms: float | None = None
    peak_rss_bytes: int | None = None
    rss_measurement_method: str | None = None

    raw_output_sha256: str | None = None
    route_artifact_sha256: str | None = None
    failure_reason: str | None = None
    warnings: list[dict[str, Any]] = field(default_factory=list)
    tool_specific: dict[str, Any] = field(default_factory=dict)
    schema_version: str = SCHEMA_VERSION

    def __post_init__(self) -> None:
        if not self.target_id or not self.target_smiles or not self.arm_id:
            raise FourToolRecordError("target_id, target_smiles, and arm_id are required")
        if self.tool not in TOOLS:
            raise FourToolRecordError(f"unsupported tool {self.tool!r}")
        if self.run_status not in RUN_STATUSES:
            raise FourToolRecordError(f"unsupported run_status {self.run_status!r}")
        if self.sample_rank < 0:
            raise FourToolRecordError("sample_rank must be non-negative")
        if self.schema_version != SCHEMA_VERSION:
            raise FourToolRecordError(f"schema_version must be {SCHEMA_VERSION!r}")
        if self.run_status == "completed" and self.route_found is None:
            raise FourToolRecordError("completed records require route_found")
        if self.run_status != "completed" and self.route_found is not None:
            raise FourToolRecordError("non-completed records must not report route_found")
        if self.common_route_parseable is True and self.route_found is not True:
            raise FourToolRecordError("a parseable common route requires route_found=true")
        if self.strict_route_to_shared_stock is True and self.common_route_parseable is not True:
            raise FourToolRecordError("a strict shared-stock pass requires a parseable route")
        if self.run_status in {"timeout", "crashed", "setup_error", "parse_error", "not_measured"}:
            if not self.failure_reason:
                raise FourToolRecordError(
                    f"{self.run_status} records require failure_reason"
                )
        if self.tool_specific and set(self.tool_specific) != {self.tool}:
            raise FourToolRecordError("tool_specific must be namespaced by the tool")

    def to_dict(self) -> dict[str, Any]:
        return asdict(self)

    def to_json_line(self) -> str:
        return json.dumps(self.to_dict(), ensure_ascii=False, sort_keys=True)


def load_records(path: str) -> list[FourToolRecord]:
    records: list[FourToolRecord] = []
    seen: set[tuple[str, str, str]] = set()
    with open(path, encoding="utf-8") as handle:
        for line_number, line in enumerate(handle, 1):
            if not line.strip():
                continue
            try:
                record = FourToolRecord(**json.loads(line))
            except (TypeError, ValueError, json.JSONDecodeError) as exc:
                raise FourToolRecordError(f"{path}:{line_number}: {exc}") from exc
            key = (record.target_id, record.tool, record.arm_id)
            validate_unique_key(key, seen)
            records.append(record)
    return records


def validate_unique_key(
    key: tuple[str, str, str], seen: set[tuple[str, str, str]]
) -> None:
    """Add a record identity to ``seen`` or reject a duplicate."""
    if key in seen:
        raise FourToolRecordError(f"duplicate record key {key!r}")
    seen.add(key)
