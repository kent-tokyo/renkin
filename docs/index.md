---
title: "RENKIN: Local Retrosynthesis Planning and Route Audit"
description: "A pure-Rust CASP planner and reproducible local route-audit layer for Python, Rust, CLI, MCP, and WebAssembly."
---

# RENKIN

**Plan routes. Audit routes. Keep the evidence local.**

RENKIN is a pure-Rust computer-aided synthesis planning (CASP) engine and a
tool-neutral route-audit layer. It searches from target SMILES to a declared
stock, then can audit routes from RENKIN, AiZynthFinder, Syntheseus, and
SynPlanner with the same deterministic checks.

Current release: **v1.0.11**.

## Start here

=== "Browser"

    [Open the local WebAssembly playground](https://kent-tokyo.github.io/renkin/playground/){ .md-button .md-button--primary }

=== "Python"

    ```bash
    pip install renkin
    ```

    ```python
    import renkin
    routes = renkin.find_routes("CC(=O)Oc1ccccc1C(=O)O", depth=5)
    ```

=== "CLI"

    ```bash
    cargo install renkin
    renkin --target "CC(=O)Oc1ccccc1C(=O)O" --depth 5
    ```

## Two complementary jobs

| Job | RENKIN does | Read next |
| --- | --- | --- |
| Plan | Searches with bounded A*/beam exploration, rule/template expansion, and declared building-block stock | [Quick start](getting_started/quickstart.md) |
| Audit | Checks route topology, structure, stock policy, optional forward replay, and evidence receipts | [Audit reproducibility](guides/audit-reproducibility-contract.md) |

The core has no C/C++ dependency and is available as a Rust crate, CLI,
Python package, MCP server, and browser WebAssembly module. Local audit inputs
and private-stock policies stay local unless an operator deliberately exports
them.

The default planner currently has 24 hand-crafted rules. Its repository stock
file contains 402 compounds; installations without that file, and WASM, use a
compiled-in 152-compound fallback. Supply an explicit stock file when this
distinction matters to a run.

## What an audit report means

An audit verdict is `pass`, `fail`, or `partial`. `partial` is intentional:
if the supplied route cannot support a check, for example because it lacks an
atom-mapped reaction representation, RENKIN records `not_evaluable` instead
of inventing a result. A route report is not an experimental-success,
yield, safety, or regulatory claim.

Useful guides:

- [Private stock policy](guides/private-stock-policy.md)
- [Chemical review rubric](guides/chemical-review-rubric.md)
- [Evidence-carrying interchange](guides/evidence-carrying-interchange.md)
- [MCP server](guides/mcp.md)

## Benchmark boundary

The registered Phase 55 TEST used a frozen 690-target cohort, shared stock,
and declared budgets. RENKIN had 481/690 strict routes (69.71%) and
AiZynthFinder 4.4.1 had 32/690 (4.64%); the paired coverage difference was
+65.07 percentage points (95% CI +61.45 to +68.55).

This supports a coverage result only for that registered configuration. It
does not establish universal planner superiority or experimental viability.
Peak-RSS and time-to-first-route receipts are still pending, so it does not
support a whole-cohort performance claim. See the [Benchmark overview](benchmark.md)
and [formal Phase 55 result record](benchmark/phase55-r2-result-20260916.md).

## Choose an interface

| Need | Interface |
| --- | --- |
| Application integration | [Python API](api/python.md) or [Rust API](api/rust.md) |
| Browser-only workflow | [WASM / JavaScript](api/wasm.md) |
| Automation agent | [MCP server](guides/mcp.md) |
| Forward validation and retrieval | [Forward tools](guides/forward-prediction.md) |

For source, releases, and issue tracking, visit
[github.com/kent-tokyo/renkin](https://github.com/kent-tokyo/renkin).
