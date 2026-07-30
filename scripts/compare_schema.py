"""PlannerComparisonRow schema v1 for the Issue #66 open-source planner comparison.

One row per (target, tool, comparison_mode). Never trusts a tool's own
self-report for timing/route-quality -- those live in `tool_specific`, kept
strictly separate from the common, cross-tool-comparable fields.

The `tool` field is a CLOSED enum: only "renkin" and "aizynthfinder" are
valid values in this round. No commercial platform must ever be
constructible here -- see the three-part test in
scripts/tests/test_compare_schema.py (set equality, deserialization
rejection, source-grep deny-list).
"""

from __future__ import annotations

import json
from dataclasses import asdict, dataclass, field

SCHEMA_VERSION = "1.0"

# Closed set -- exact equality is asserted in tests, not just "these are present".
VALID_TOOLS = frozenset({"renkin", "aizynthfinder"})

VALID_COMPARISON_MODES = frozenset({"native", "matched_stock"})

VALID_RUN_STATUSES = frozenset(
    {"completed", "timeout", "crashed", "invalid_input", "setup_error"}
)

# Fields that must be null when run_status != "completed" and no route was
# produced. Enforced by validate_row(), not left to convention.
_ROUTE_DEPENDENT_FIELDS = (
    "tool_reported_route_count",
    "time_to_first_route_ms",
    "best_route_depth",
    "best_route_step_count",
)
_TREE_DEPENDENT_FIELDS = (
    "best_route_leaf_count",
    "all_leaves_in_configured_stock",
    "reaction_steps_parseable",
    "common_mass_conservation_status",
    "normalized_route_sha256",
)


class SchemaValidationError(ValueError):
    pass


def validate_tool(tool: str) -> None:
    if tool not in VALID_TOOLS:
        raise SchemaValidationError(
            f"invalid tool {tool!r}: must be one of {sorted(VALID_TOOLS)}"
        )


def validate_comparison_mode(mode: str) -> None:
    if mode not in VALID_COMPARISON_MODES:
        raise SchemaValidationError(
            f"invalid comparison_mode {mode!r}: must be one of {sorted(VALID_COMPARISON_MODES)}"
        )


def validate_run_status(status: str) -> None:
    if status not in VALID_RUN_STATUSES:
        raise SchemaValidationError(
            f"invalid run_status {status!r}: must be one of {sorted(VALID_RUN_STATUSES)}"
        )


@dataclass
class PlannerComparisonRow:
    target_id: str
    target_smiles: str
    sample_rank: int
    tool: str
    tool_version: str
    configuration_id: str
    comparison_mode: str
    run_status: str

    route_found: bool | None = None
    tool_reported_route_count: int | None = None
    time_to_first_route_ms: float | None = None
    total_elapsed_ms: float | None = None
    peak_rss_bytes: int | None = None
    rss_measurement_method: str | None = None

    best_route_depth: int | None = None
    best_route_step_count: int | None = None
    best_route_leaf_count: int | None = None
    all_leaves_in_configured_stock: bool | None = None
    route_tree_parseable: bool | None = None
    reaction_steps_parseable: bool | None = None
    common_mass_conservation_status: str | None = None  # "balanced"|"imbalanced"|"not_evaluable"

    common_validation_warnings: list = field(default_factory=list)
    adapter_warnings: list = field(default_factory=list)

    raw_output_sha256: str | None = None
    normalized_route_sha256: str | None = None

    tool_specific: dict = field(default_factory=dict)

    schema_version: str = SCHEMA_VERSION

    def __post_init__(self) -> None:
        validate_tool(self.tool)
        validate_comparison_mode(self.comparison_mode)
        validate_run_status(self.run_status)
        if self.tool not in self.tool_specific and self.tool_specific:
            # tool_specific must be namespaced under the tool's own key.
            raise SchemaValidationError(
                f"tool_specific must be namespaced under {{'{self.tool}': {{...}}}}, "
                f"got top-level keys {sorted(self.tool_specific.keys())}"
            )

    def to_dict(self) -> dict:
        d = asdict(self)
        return d

    def to_json_line(self) -> str:
        return json.dumps(self.to_dict(), sort_keys=True)


def validate_row_nullability(row: PlannerComparisonRow) -> list[str]:
    """Returns a list of nullability-contract violations (empty if none).

    Does not raise -- callers (e.g. a --strict adapter mode) decide whether
    a violation is fatal.
    """
    problems: list[str] = []

    if row.run_status == "setup_error":
        for f in ("total_elapsed_ms", "peak_rss_bytes", "raw_output_sha256"):
            if getattr(row, f) is not None:
                problems.append(f"{f} must be null when run_status='setup_error'")

    if row.run_status != "completed":
        if row.route_found is not None:
            problems.append("route_found must be null unless run_status='completed'")
    else:
        if row.route_found is None:
            problems.append("route_found must be set (true/false) when run_status='completed'")

    if row.route_found is not True:
        for f in _ROUTE_DEPENDENT_FIELDS:
            if getattr(row, f) is not None:
                problems.append(f"{f} must be null when route_found is not true")

    if row.route_tree_parseable is not True:
        for f in _TREE_DEPENDENT_FIELDS:
            if getattr(row, f) is not None:
                problems.append(f"{f} must be null unless route_tree_parseable is true")

    return problems


def load_rows(path: str) -> list[PlannerComparisonRow]:
    rows = []
    with open(path, "r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            d = json.loads(line)
            d.pop("schema_version", None)
            rows.append(PlannerComparisonRow(**d))
    return rows
