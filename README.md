# RENKIN

Retrosynthesis planning and route auditing in Rust.

[![CI](https://github.com/kent-tokyo/renkin/actions/workflows/ci.yml/badge.svg?branch=master)](https://github.com/kent-tokyo/renkin/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/renkin.svg)](https://crates.io/crates/renkin)
[![PyPI](https://img.shields.io/pypi/v/renkin.svg)](https://pypi.org/project/renkin/)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

[Documentation](https://kent-tokyo.github.io/renkin/) · [Playground](https://kent-tokyo.github.io/renkin/playground/) · [日本語版](README_ja.md) · [中文](README_zh.md)

RENKIN has two uses:

- **Planner:** search retrosynthetic routes from a target molecule to building blocks.
- **Bridge:** audit routes from RENKIN, AiZynthFinder, Syntheseus, or SynPlanner.

Auditing is local and reproducible. Reports include structural checks, stock
coverage, forward replay, provenance, and a verifiable audit manifest.

## Install

```bash
pip install renkin
cargo add renkin
npm install renkin
```

For Syntheseus support:

```bash
pip install 'renkin[syntheseus]'
```

## Audit a route

```python
import json
import renkin

report = json.loads(
    renkin.audit_route(open("trees.json").read(), format="aizynthfinder")
)
print(report["summary"])
```

Use `format="syntheseus"`, `format="synplanner"`, or `format="renkin"` for
other supported route formats. The same audit pipeline is used for every
source.

```bash
renkin audit-route route.json --format auto --output json
```

Optional private stock and policy checks remain local:

```bash
renkin audit-route route.json \
  --private-stock private-vendors.csv \
  --stock-policy private-policy.json --output json
```

See the [audit guide](https://kent-tokyo.github.io/renkin/guides/audit-reproducibility-contract/),
[private stock policy](https://kent-tokyo.github.io/renkin/guides/private-stock-policy/),
and [route interchange](https://kent-tokyo.github.io/renkin/guides/evidence-carrying-interchange/).

## Plan a route

```python
import json
import renkin

result = json.loads(renkin.find_routes(
    target="CC(=O)Oc1ccccc1C(=O)O",  # aspirin
    depth=5,
    max_routes=3,
))

for route in result["routes"]:
    for step in route["steps"]:
        print(step["target"], "→", " + ".join(step["precursors"]))
```

CLI:

```bash
cargo run --release -- \
  --target "CC(=O)Oc1ccccc1C(=O)O" \
  --depth 5 --beam-width 100 --format tree
```

The planner uses A*/AND-OR search, template indexing, beam limits,
stock-aware scoring, and forward validation. See the [API documentation](https://docs.rs/renkin)
and [retrosynthesis guide](https://kent-tokyo.github.io/renkin/guides/rust-retrosynthesis/).

## Components

| Component | Purpose |
| --- | --- |
| `renkin` | Planner, CLI, Python bindings, and WASM module |
| `renkin-forward` | Forward prediction, enumeration, hints, and validation |
| `renkin-kg` | Reaction knowledge-graph export |
| `renkin-mcp` | Local MCP server for search and audit |

The chemistry layer is [`chematic`](https://docs.rs/chematic/), with no
C/C++ dependency in the core.

## MCP

```bash
cargo run --release --bin renkin-mcp
```

The MCP server communicates over stdio and exposes search, validation,
explanation, constraints, diagnostics, and audit receipts. See the [MCP guide](https://kent-tokyo.github.io/renkin/guides/mcp/).

## Benchmark status

The latest checked-in formal comparison uses a declared shared-stock endpoint.
RENKIN and AiZynthFinder reached the same route count in the published
equal-condition comparison; universal CASP superiority is not proven.
Success rate, speed, stock definition, validation, and route quality must be
reported separately.

See the [benchmark documentation](https://kent-tokyo.github.io/renkin/benchmark/)
for the full measurements, methodology, confidence intervals, and limitations.

## Development

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Read [`AGENTS.md`](AGENTS.md), [`tasks/lessons.md`](tasks/lessons.md), and
the [roadmap](ROADMAP.md) before changing the chemistry or search core.

Important boundaries:

- stock identity is exact standardized canonical-SMILES membership;
- route success is not experimental success;
- external model output is evidence or candidates, not automatic validity;
- benchmark claims must name the dataset, stock, versions, and endpoint.

## License

MIT. See [LICENSE](LICENSE).
