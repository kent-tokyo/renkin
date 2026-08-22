"""Typed wrapper around ``renkin.audit_route``'s JSON string output.

``renkin.audit_route(...) -> str`` stays exactly as it is -- this module
never touches it, never changes its behavior. ``audit_route_report(...)``
below is a thin pure-Python layer on top: call the existing string API,
``json.loads()`` it, and hand back attribute-accessible dataclasses
instead of a dict-of-dicts. No Rust code is involved.

Every dataclass field here is named and typed to match the real JSON
shape emitted by ``bridge::build_audit_route_report_with_policy`` (see
``docs/guides/audit-reproducibility-contract.md`` for the wire-level
contract). One deliberate simplification: several fields are *absent*
from the JSON entirely when not applicable (Rust's
``#[serde(skip_serializing_if = "Option::is_none")]``), while others
serialize as an explicit JSON ``null``. Both cases collapse to ``None``
here -- a missing key and an explicit null both mean "not present" to a
caller of this convenience API. Anyone who needs to distinguish
wire-level absent-vs-null should use ``audit_route()`` (the string API)
and inspect the raw JSON directly; that distinction is never lost, only
not surfaced by this typed layer.

Enum-shaped values (``status``, ``source``, finding ``code``, etc.) are
typed as plain ``str``, matching the wire format exactly -- not Python
``Enum`` classes. A real ``Enum`` would break the moment a future RENKIN
version ships a new variant string this module doesn't know about yet; a
plain ``str`` degrades gracefully instead.
"""

import json
from dataclasses import dataclass, field
from typing import List, Optional

__all__ = [
    "AuditFinding",
    "ForwardValidationResult",
    "StockValidationResult",
    "AuditedStep",
    "AuditReport",
    "AuditManifest",
    "AuditRouteSummary",
    "AuditRouteReport",
    "audit_route_report",
]


@dataclass
class AuditFinding:
    code: str
    severity: str
    node: Optional[str] = None


@dataclass
class ForwardValidationResult:
    status: str
    method: str
    reason: Optional[str] = None


@dataclass
class StockValidationResult:
    status: str
    reason: Optional[str] = None


@dataclass
class AuditedStep:
    target: str
    precursors: List[str]
    forward_validation: ForwardValidationResult


@dataclass
class AuditReport:
    """A single audited route (nested inside ``AuditRouteReport.routes``).

    Matches the Rust ``AuditReport`` struct (``src/bridge/audit.rs``) name
    for name -- a different, unrelated ``AuditReport`` class exists in
    ``scripts/ord_evidence_audit.py``, but that module is never imported
    alongside ``renkin``, so there is no real name collision.
    """

    source: str
    status: str
    route_tree_parseable: bool
    reaction_steps_parseable: Optional[bool] = None
    stock_validation: Optional[StockValidationResult] = None
    target_element_accounting_status: Optional[str] = None
    normalized_route_sha256: Optional[str] = None
    steps: List[AuditedStep] = field(default_factory=list)
    findings: List[AuditFinding] = field(default_factory=list)


@dataclass
class AuditManifest:
    renkin_version: str
    report_schema_version: int
    source_format: str
    input_sha256: str
    policy: str
    source_version: Optional[str] = None
    stock_sha256: Optional[str] = None


@dataclass
class AuditRouteSummary:
    routes_total: int
    passed: int
    fail: int
    partial: int


@dataclass
class AuditRouteReport:
    schema_version: int
    source_format: str
    audit_manifest: AuditManifest
    summary: AuditRouteSummary
    routes: List[AuditReport]


def _finding_from_json(data: dict) -> AuditFinding:
    return AuditFinding(code=data["code"], severity=data["severity"], node=data.get("node"))


def _forward_validation_from_json(data: dict) -> ForwardValidationResult:
    return ForwardValidationResult(
        status=data["status"], method=data["method"], reason=data.get("reason")
    )


def _stock_validation_from_json(data: Optional[dict]) -> Optional[StockValidationResult]:
    if data is None:
        return None
    return StockValidationResult(status=data["status"], reason=data.get("reason"))


def _step_from_json(data: dict) -> AuditedStep:
    return AuditedStep(
        target=data["target"],
        precursors=list(data["precursors"]),
        forward_validation=_forward_validation_from_json(data["forward_validation"]),
    )


def _route_from_json(data: dict) -> AuditReport:
    return AuditReport(
        source=data["source"],
        status=data["status"],
        route_tree_parseable=data["route_tree_parseable"],
        reaction_steps_parseable=data.get("reaction_steps_parseable"),
        stock_validation=_stock_validation_from_json(data.get("stock_validation")),
        target_element_accounting_status=data.get("target_element_accounting_status"),
        normalized_route_sha256=data.get("normalized_route_sha256"),
        steps=[_step_from_json(s) for s in data["steps"]],
        findings=[_finding_from_json(f) for f in data["findings"]],
    )


def _manifest_from_json(data: dict) -> AuditManifest:
    return AuditManifest(
        renkin_version=data["renkin_version"],
        report_schema_version=data["report_schema_version"],
        source_format=data["source_format"],
        input_sha256=data["input_sha256"],
        policy=data["policy"],
        source_version=data.get("source_version"),
        stock_sha256=data.get("stock_sha256"),
    )


def _summary_from_json(data: dict) -> AuditRouteSummary:
    return AuditRouteSummary(
        routes_total=data["routes_total"],
        passed=data["pass"],
        fail=data["fail"],
        partial=data["partial"],
    )


def _report_from_json(data: dict) -> AuditRouteReport:
    return AuditRouteReport(
        schema_version=data["schema_version"],
        source_format=data["source_format"],
        audit_manifest=_manifest_from_json(data["audit_manifest"]),
        summary=_summary_from_json(data["summary"]),
        routes=[_route_from_json(r) for r in data["routes"]],
    )


def audit_route_report(
    content: str, format: str = "auto", stock_text: str = "", policy: str = "standard"
) -> AuditRouteReport:
    """Typed counterpart to ``audit_route()``: same arguments, same
    validation, same errors (``ValueError`` on malformed input or an
    invalid ``format``/``policy`` string) -- the only difference is the
    return type. See the module docstring for the absent-vs-null and
    enum-as-``str`` design notes.
    """
    from . import audit_route as _audit_route_str

    raw = _audit_route_str(content, format=format, stock_text=stock_text, policy=policy)
    return _report_from_json(json.loads(raw))
