# RENKIN

Local retrosynthesis planning and route auditing in Rust.

[![CI](https://github.com/kent-tokyo/renkin/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/kent-tokyo/renkin/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/renkin.svg)](https://crates.io/crates/renkin)
[![PyPI](https://img.shields.io/pypi/v/renkin.svg)](https://pypi.org/project/renkin/)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

[Docs](https://kent-tokyo.github.io/renkin/) · [Playground](https://kent-tokyo.github.io/renkin/playground/) · [日本語](README_ja.md) · [中文](README_zh.md)

RENKIN has two complementary jobs:

- **Plan:** search retrosynthetic routes from target SMILES to declared building blocks.
- **Audit:** apply the same deterministic structural, stock, forward-replay, and provenance checks to routes from RENKIN, AiZynthFinder, Syntheseus, or SynPlanner.

Current release: **v1.0.11**. Core chemistry is pure Rust; the project ships a CLI, Rust crate, Python package, MCP server, and browser WebAssembly module.
The default planner has 24 hand-crafted rules. Repository-backed runs use the
402-compound stock file; WASM and installations without that file use a
compiled-in 152-compound fallback. Supply stock explicitly when it matters.

## Install

```bash
pip install renkin
cargo add renkin
npm install renkin
```

## Start in two commands

Plan aspirin from the CLI:

```bash
renkin --target "CC(=O)Oc1ccccc1C(=O)O" --depth 5 --beam-width 100
```

Audit an exported route locally:

```bash
renkin audit-route route.json --format auto --output json
```

Python uses the same engine:

```python
import json, renkin
routes = json.loads(renkin.find_routes("CC(=O)Oc1ccccc1C(=O)O", depth=5))
report = json.loads(renkin.audit_route(open("route.json").read(), format="auto"))
```

## Choose an interface

| Need | Start here |
| --- | --- |
| Browser-only exploration or audit | [Playground](https://kent-tokyo.github.io/renkin/playground/) · [WASM API](https://kent-tokyo.github.io/renkin/api/wasm/) |
| Application integration | [Python](https://kent-tokyo.github.io/renkin/api/python/) · [Rust](https://docs.rs/renkin) |
| Local agent workflow | [MCP guide](https://kent-tokyo.github.io/renkin/guides/mcp/) |
| Private stock or route evidence | [Audit guide](https://kent-tokyo.github.io/renkin/guides/audit-reproducibility-contract/) · [Policy guide](https://kent-tokyo.github.io/renkin/guides/private-stock-policy/) |

`renkin capabilities`, `renkin.capabilities()`, WASM `capabilities()`, and modern MCP discovery expose each surface's effective limits. Private inputs stay local unless an operator explicitly exports them.

## Scope and benchmark boundary

An audit result is evidence, not a claim that a synthesis will succeed in the laboratory. Stock identity is exact standardized canonical-SMILES membership; model output and route imports are never accepted without validation.

The registered Phase 55 shared-stock TEST found strict routes for RENKIN on 481/690 targets (69.71%) and AiZynthFinder 4.4.1 on 32/690 (4.64%), under its pinned cohort, stock, assets, and budgets. It does **not** establish universal planner superiority, laboratory viability, or whole-cohort speed superiority. Read the [result record](https://kent-tokyo.github.io/renkin/benchmark/phase55-r2-result-20260916/) before reusing the figure.

## Development

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

For architecture and contribution rules, see [AGENTS.md](AGENTS.md), [CONTRIBUTING.md](CONTRIBUTING.md), and the current [ROADMAP.md](ROADMAP.md). Full release history remains in [CHANGELOG.md](CHANGELOG.md).

## License

MIT. See [LICENSE](LICENSE).
