"""Type stubs for the ``renkin`` PyO3 extension module.

Picked up automatically by maturin (a root-level ``<module-name>.pyi`` next
to a pure-extension-layout ``pyproject.toml``) and included in the wheel
alongside a generated ``py.typed`` marker -- no ``pyproject.toml`` change
needed. Every function here returns a JSON *string*; parse it yourself with
``json.loads()``. See ``docs/api/python.md`` for the full field-by-field
return-shape documentation this stub deliberately doesn't duplicate.
"""

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
