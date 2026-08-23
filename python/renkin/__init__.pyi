"""Type stubs for the ``renkin`` package's compiled functions.

Lives at ``python/renkin/__init__.pyi``, stubbing ``renkin/__init__.py``'s
``from .renkin import *`` re-export of the PyO3 extension submodule
(mixed Rust/Python maturin layout, since v0.30.0 added a pure-Python
``renkin.syntheseus_exporter`` alongside the compiled bindings). Every
compiled function here returns a JSON *string*; parse it yourself with
``json.loads()``. See ``docs/api/python.md`` for the full field-by-field
return-shape documentation this stub deliberately doesn't duplicate.
``audit_route_report`` (v0.32.0, pure Python, defined in
``audit_report.py``) is the one exception -- it returns the typed
``AuditRouteReport`` dataclass directly.
"""

from .audit_report import (
    AuditedStep as AuditedStep,
    AuditFinding as AuditFinding,
    AuditManifest as AuditManifest,
    AuditReport as AuditReport,
    AuditRouteReport as AuditRouteReport,
    AuditRouteSummary as AuditRouteSummary,
    ForwardValidationResult as ForwardValidationResult,
    StockValidationResult as StockValidationResult,
)

__version__: str

def find_routes(
    target: str,
    depth: int = 5,
    max_routes: int = 5,
    beam_width: int = 0,
    building_blocks: list[str] | None = None,
    avoid_elements: str = "",
    require_elements: str = "",
    verbose: bool = False,
    bb_prices_path: str | None = None,
    templates_path: str | None = None,
    template_metadata_path: str | None = None,
    reranker_model_path: str | None = None,
    reranker_freq_table_path: str | None = None,
    top_templates: int | None = None,
    search_mode: str = "standard",
    coverage_templates_path: str | None = None,
    coverage_timeout_seconds: int | None = None,
    search_diagnostics: bool = False,
) -> str: ...
def predict_forward(
    reactants: list[str],
    templates_path: str | None = None,
    max_results: int = 5,
) -> str: ...
def validate_forward(
    route_json: str,
    templates_path: str | None = None,
    max_results: int = 5,
) -> str: ...
def audit_route(
    content: str,
    format: str = "auto",
    stock_text: str = "",
    policy: str = "standard",
) -> str: ...
def audit_route_report(
    content: str,
    format: str = "auto",
    stock_text: str = "",
    policy: str = "standard",
) -> AuditRouteReport: ...
