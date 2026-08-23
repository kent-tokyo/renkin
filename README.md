# RENKIN — Retrosynthesis Engine for Knowledge-Informed Navigation

> **Computer-Aided Synthesis Planning (CASP) · Pure Rust · WebAssembly · Python**  
> Named after 錬金 (れんきん, *renkin*) — Japanese for alchemy: just as alchemists transformed base metals into gold, RENKIN transforms target molecules back into cheap starting materials.

<p>
  <a href="https://github.com/kent-tokyo/renkin/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/kent-tokyo/renkin/actions/workflows/ci.yml/badge.svg?branch=master"></a>
  <a href="https://github.com/kent-tokyo/renkin/actions/workflows/docs.yml"><img alt="Docs" src="https://github.com/kent-tokyo/renkin/actions/workflows/docs.yml/badge.svg?branch=master"></a>
</p>

<p>
  <a href="https://crates.io/crates/renkin"><img alt="Crates.io" src="https://img.shields.io/crates/v/renkin.svg"></a>
  <a href="https://docs.rs/renkin"><img alt="docs.rs" src="https://docs.rs/renkin/badge.svg"></a>
  <a href="https://pypi.org/project/renkin/"><img alt="PyPI" src="https://img.shields.io/pypi/v/renkin.svg"></a>
  <a href="https://pypi.org/project/renkin/"><img alt="Python" src="https://img.shields.io/pypi/pyversions/renkin.svg"></a>
  <a href="https://www.npmjs.com/package/renkin"><img alt="npm" src="https://img.shields.io/npm/v/renkin.svg"></a>
  <a href="LICENSE"><img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-blue.svg"></a>
</p>

[日本語版 README](./README_ja.md) · [中文版 README](./README_zh.md) · [**Documentation**](https://kent-tokyo.github.io/renkin/) · [**Live Demo →**](https://kent-tokyo.github.io/renkin/playground/)

---

**Keep your planner. Audit every route.** Audit retrosynthesis routes from AiZynthFinder, Syntheseus, SynPlanner, or RENKIN — locally, reproducibly, and without sending molecular structures anywhere.

[**Audit a route in your browser →**](https://kent-tokyo.github.io/renkin/playground/) · [**Python quick start ↓**](#audit-a-route) · [**Route planning engine ↓**](#quick-start)

---

## What is RENKIN?

RENKIN Bridge is a tool-neutral **route auditor**: it checks structural integrity, stock coverage, and declared-reaction forward-replay for routes from **AiZynthFinder**, **Syntheseus**, **SynPlanner**, or RENKIN's own planner — the identical `pass`/`fail`/`partial` pipeline regardless of which tool produced the route, entirely local and reproducible (every audit records a verifiable [`audit_manifest`](https://kent-tokyo.github.io/renkin/guides/audit-reproducibility-contract/)), structures never leaving your machine unless you explicitly ask.

RENKIN is also, in its own right, an open-source **retrosynthesis engine** for **computer-aided synthesis planning (CASP)** that automatically discovers chemical reaction routes from a target molecule back to cheap, commercially available starting materials.

Built entirely in Rust with the [`chematic`](https://docs.rs/chematic/) cheminformatics crate — zero C/C++ dependencies, `#![forbid(unsafe_code)]` throughout. One codebase compiles to a native CLI, a Rust library, Python wheels (PyO3), and a WebAssembly module that runs entirely client-side in the browser.

---

## Installation

```bash
pip install renkin          # Python
cargo add renkin            # Rust
npm install renkin          # JavaScript (browser / bundler -- see docs/api/wasm.md)
```

Auditing Syntheseus routes needs one more, optional package:
`pip install renkin[syntheseus]` (verified against Syntheseus `0.7.2` and
`0.8.0` — see the [compatibility spike](https://github.com/kent-tokyo/renkin/blob/master/docs/design/syntheseus-0.8-compatibility-spike.md)).

---

## Live Playground

**[→ Try it now](https://kent-tokyo.github.io/renkin/playground/)** — runs entirely in WebAssembly: no installation, no server, no network calls.

---

## Audit a Route

Bring a route from wherever you already plan them — every path below runs through the identical audit pipeline: the same `pass`/`fail`/`partial` verdict regardless of source tool, or whether you ran it from the CLI, Python, or a browser tab.

**AiZynthFinder**

```python
import json
import renkin

report = json.loads(
    renkin.audit_route(open("trees.json").read(), format="aizynthfinder")
)
print(report["summary"])
```

**Syntheseus** (`pip install renkin[syntheseus]`)

```python
import json
import renkin
from renkin.syntheseus_exporter import dumps_syntheseus_route_v1

route_json = dumps_syntheseus_route_v1(my_synthesis_graph)
report = json.loads(renkin.audit_route(route_json, format="syntheseus"))
print(report["summary"])
```

**SynPlanner**

```python
import json
import renkin

report = json.loads(
    renkin.audit_route(open("routes.json").read(), format="synplanner")
)
print(report["summary"])
```

**In your browser** — no installation, no upload, no server: [**Try the Playground →**](https://kent-tokyo.github.io/renkin/playground/)

Full walkthroughs with real output, end to end: [AiZynthFinder](https://kent-tokyo.github.io/renkin/guides/aizynthfinder-audit-demo/) · [Syntheseus](https://kent-tokyo.github.io/renkin/guides/syntheseus-audit-demo/) · [SynPlanner](https://kent-tokyo.github.io/renkin/guides/synplanner-audit-demo/).

---

## Quick Start

*Planning a route from scratch, not auditing one you already have — see [Audit a Route](#audit-a-route) above for that.*

```python
import json
import renkin

result = json.loads(
    renkin.find_routes(
        target="CC(=O)Oc1ccccc1C(=O)O",  # Aspirin
        depth=5,
        max_routes=3,
    )
)

for route in result["routes"]:
    for step in route["steps"]:
        print(f"  {step['target']} → {' + '.join(step['precursors'])}  [{step['rule']}]")
```

```javascript
import init, { find_routes } from './pkg/renkin.js';
await init();
const result = JSON.parse(find_routes("CC(=O)Oc1ccccc1C(=O)O", 5, 3, 0));
```

```bash
./target/release/renkin --target "CC(=O)Oc1ccccc1C(=O)O" --depth 5 \
    --templates data/templates_extracted_5000.smi --format tree
```

```text
Target: CC(=O)Oc1ccccc1C(=O)O
Routes found: 3

Route 1  [score=1.02, depth=1]
OC(=O)c1ccccc1OC(=O)C
└── [extracted_169]
    ├── OC(=O)C  ✓ BB
    └── [OH]c1ccccc1C(=O)O  ✓ BB

Route 2  [score=1.02, depth=1]
OC(=O)c1ccccc1OC(=O)C
└── [extracted_145]
    ├── CC(=O)Cl  ✓ BB
    └── [OH]c1ccccc1C(=O)O  ✓ BB

Route 3  [score=1.03, depth=1]
OC(=O)c1ccccc1OC(=O)C
└── [extracted_238]
    ├── c1cccc(c1O)C(O)=O  ✓ BB
    └── C([OH])(=O)C  ✓ BB
```

Use `--format mermaid` for GitHub/Notion-compatible flowcharts.

[![Open In Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/github/kent-tokyo/renkin/blob/master/examples/renkin_quickstart.ipynb)

---

## Current Limitations

⚠️ Benchmark numbers are under active re-measurement after a validator-accuracy
fix — historical 78.0%/95.9%/81.8%(ChEMBL) figures elsewhere in this repo predate
that fix and are invalidated. RENKIN does not predict yields, calibrated
experimental success probabilities, or side reactions, and does not search
the literature automatically (`success_probability` is a template-frequency
search-ranking score, not a calibrated prediction — see
[Benchmark](https://kent-tokyo.github.io/renkin/benchmark/) for the corrected
historical baseline, full methodology, and known limitations — that page is a
frozen, single-commit measurement, not a live number).

---

## Why RENKIN?

RENKIN is designed as a Rust-native synthesis planning stack:

| | |
|---|---|
| **Fast** | A\* / AND-OR tree search with beam search and template frequency weighting |
| **Portable** | Native CLI · Python wheels · npm/WASM · browser playground — one codebase |
| **Explainable** | Per-step `confidence`, `atom_economy`, `route_cost`, and `procedure_hint` |
| **Verifiable** | `renkin-forward` validates each retrosynthetic step by forward-applying templates |
| **Benchmarkable** | USPTO-50k, PaRoutes-style evaluation, route diversity, and atom balance checks |
| **Agent-ready** | MCP server exposes routes and validation to Claude Desktop and AI agents |

---

## Constraint-based Search

Restrict routes by the element composition of their building blocks.

**Default search** — all 5 routes for biphenyl:

```bash
renkin --target "c1ccc(-c2ccccc2)cc1" --templates data/templates_extracted_5000.smi --format tree
```

```text
Routes found: 5
Route 1  [score=1.00, depth=1]  c1ccccc1Br + c1c(B(O)O)cccc1
Route 2  [score=1.03, depth=1]  c1ccccc1Br + c1c(B(O)O)cccc1
Route 3  [score=1.06, depth=1]  c1cc(Cl)ccc1 + c1c(B(O)O)cccc1
Route 4  [score=1.08, depth=1]  c1(I)ccccc1  + c1c(B(O)O)cccc1
Route 5  [score=1.08, depth=1]  c1ccccc1Br  + c1(B2OC(C(C)(C)O2)(C)C)ccccc1
```

**Constrained search** — boronic-acid coupling, no Br or I starting materials:

```bash
renkin --target "c1ccc(-c2ccccc2)cc1" --templates data/templates_extracted_5000.smi \
    --require-elements "B" --avoid-elements "Br,I" --format tree
```

```text
Routes found: 1

Route 1  [score=1.06, depth=1]
c1ccccc1-c2ccccc2
└── [extracted_398]
    ├── c1cc(Cl)ccc1  ✓ BB
    └── c1c(B(O)O)cccc1  ✓ BB
```

Constraints compose freely and are enforced in two layers:
- `--avoid-elements` **prunes expansions during search** when a BB precursor contains a forbidden element (no dead-end nodes added to the heap).
- A final route-level post-filter is still applied for correctness.
- `--require-elements` is a route-level post-filter only.

Add `--verbose` to print search statistics (nodes expanded, elapsed time) to stderr. Performance counters are available in native builds only; disabled in WASM.

Add `--search-diagnostics` to add a `search_diagnostics` block to JSON output (beam eviction counts/scores, cross-template duplicate precursor signatures, rule-application attempts, stock-terminal/non-stock candidate counts, depth-wise branching factor, hypothetical dedup-strategy counts) — diagnostics-only bookkeeping, off by default, does not affect search behavior ([Issue #101](https://github.com/kent-tokyo/renkin/issues/101)). Add `--candidate-trace-limit <N>` (implies `--search-diagnostics`) to also collect up to `N` per-candidate trace records (precursor signature, template provenance, beam rank/survival, whether it fed a returned route) — offline diagnostic use only, gated at collection time so the default no-trace path allocates nothing extra.

---

## Template Evidence Metadata

Extracted templates only have a positional display name (`extracted_{i}`) that
changes whenever the source `.smi` file is reordered or re-extracted, so
external knowledge (a DOI, a reported yield, a known side reaction) can't be
durably attached to one. Every template — hand-crafted and extracted — now
has a stable `template_id` instead:

- Hand-crafted rules: `rule:<rule_name>` (e.g. `rule:suzuki_retro`).
- Extracted templates: `smirks-sha256:<hex>` — the SHA-256 hex digest of the
  *trimmed* SMIRKS string. Independent of file position, load order, and
  count; purely syntactic (no SMIRKS canonicalization — a semantically
  equivalent SMIRKS written differently gets a different ID).

Run `renkin template ids <file.smi>` to list every template's `template_id`,
display name, SMIRKS, and weight (TSV by default, `--format json` for JSON) —
use this to look up the IDs you need when authoring a sidecar file.

Attach curated evidence with `--template-metadata sidecar.json` (also
available in Python as `find_routes(..., template_metadata_path=...)`),
keyed by `template_id`:

```json
{
  "schema_version": 1,
  "templates": {
    "smirks-sha256:ef8778a2888469d619c52cce7e74f6848e101049050dd1b765b78f32e3c94498": {
      "references": [
        { "id": "ref-1", "kind": "doi", "identifier": "10.xxxx/example" }
      ],
      "condition_candidates": [
        {
          "catalysts": ["Pd(PPh3)4"],
          "bases": ["K2CO3"],
          "solvents": ["EtOH", "water"],
          "temperature_c": { "min": 75.0, "max": 85.0 },
          "source": "literature",
          "scope": "template",
          "reference_ids": ["ref-1"]
        }
      ],
      "reported_yields": [
        {
          "percentage": { "min": 72.0, "max": 81.0 },
          "basis": "isolated",
          "source": "literature",
          "scope": "template",
          "reference_ids": ["ref-1"]
        }
      ],
      "warnings": [
        {
          "code": "possible_protodeboronation",
          "severity": "medium",
          "message": "Protodeboronation has been reported under prolonged aqueous heating.",
          "source": "literature",
          "scope": "template",
          "reference_ids": ["ref-1"]
        }
      ]
    }
  }
}
```

A matching step gets an `evidence` field with `condition_candidates`,
`reported_yields`, `references`, and `warnings`; steps whose template has no
sidecar entry get no `evidence` key at all. The sidecar is loaded and
validated (schema version, duplicate/dangling reference IDs, yield range,
range `min <= max`, non-empty DOI/patent identifiers) **before search
starts** — malformed metadata is a hard error, and a `template_id` in the
sidecar that matches no loaded rule prints a warning rather than failing
silently.

**What this is not:**
- `reported_yields` is a curated record of what was reported externally —
  **not a RENKIN prediction**. `step_confidence`/`success_probability` are
  unaffected and keep meaning template-frequency-derived search-ranking
  scores, not experimental success rates.
- `warnings` reflects only what's explicitly present in the sidecar you
  supply — **not** automatic side-reaction detection.
- Templates without a matching sidecar entry get no fabricated evidence.
  Nothing is invented for missing data.

Yield/success prediction and automatic literature search are explicitly out
of scope for this phase — tracked as future work in
[#41](https://github.com/kent-tokyo/renkin/issues/41).

### Substrate-specific examples (`schema_version: 2`)

Everything above is *template-level*: it applies to every step using that
template, regardless of the actual molecule. `schema_version: 2` adds
`examples` — a per-template array where each entry is one curated record of
*this exact reaction*, keyed by `target_smiles`/`precursor_smiles`:

```json
{
  "schema_version": 2,
  "templates": {
    "smirks-sha256:...": {
      "references": [{ "id": "ref-1", "kind": "doi", "identifier": "10.xxxx/example" }],
      "examples": [{
        "id": "ex-1",
        "target_smiles": "c1ccc(-c2ccccc2)cc1",
        "precursor_smiles": ["Brc1ccccc1", "c1ccccc1"],
        "conditions": { "catalysts": ["Pd(PPh3)4"], "solvents": ["EtOH"], "source": "literature", "scope": "substrate_specific", "reference_ids": ["ref-1"] },
        "reported_yield": { "percentage": 78.0, "basis": "isolated", "source": "literature", "scope": "substrate_specific", "reference_ids": ["ref-1"] },
        "reference_ids": ["ref-1"]
      }]
    }
  }
}
```

`examples` requires `schema_version: 2` (a hard error under `1`); under
`schema_version: 2`, reported yields must live under `examples[].reported_yield`
too — a non-empty template-level `reported_yields` is a hard error there (it
stays allowed under `schema_version: 1`), so a substrate-specific number can't
leak onto every step using that template. Every condition/yield/warning
nested inside an example must be scoped `substrate_specific`.

A route step's `evidence.examples` are **resolved**, not just copied from the
sidecar: matched against that step by canonical target SMILES plus the
canonical, order-independent precursor set (reordering `precursor_smiles` in
the sidecar changes nothing), with every exact-substrate match kept and
same-template-different-substrate precedents capped at 3. Each resolved entry
carries a `match_kind` (`exact_substrate`/`template_only`) in the JSON itself,
plus a `template_examples_total` count — so JSON/Python consumers, not just
`--format explain`, can tell "evidence for this exact reaction" apart from
"literature precedent for a different substrate." `--format explain` shows
exact-substrate matches first, each labeled either `Exact substrate example:`
or *"different substrate; not a prediction"*, with `conditions`/
`reported_yield`/`warnings` each showing their own cited references directly
underneath (deduplicated when the same reference backs more than one part of
an example). See [Reaction Evidence guide](docs/guides/reaction-evidence.md#substrate-specific-examples-schema_version-2)
for full matching/validation semantics.

**Importing evidence from ORD.** `renkin evidence match` (exact-set batch
template matching, no fuzzy/similarity matching) and
[`scripts/ord_evidence_audit.py`](scripts/README_ord_evidence.md) (offline,
network-free) turn a locally-downloaded
[Open Reaction Database](https://github.com/open-reaction-database/ord-data)
corpus into a `schema_version: 2` sidecar — every accepted record is
independently re-validated by RENKIN's own loader, and anything not uniquely
matched, unambiguous, and provenanced is excluded and counted in an audit
report rather than guessed at. RENKIN itself never fetches or searches the
literature; reported yields are citations, not predictions. ORD's reaction
data is CC-BY-SA-4.0, a different license from RENKIN's own MIT code — see
[Reaction Evidence guide](docs/guides/reaction-evidence.md#importing-from-ord-open-reaction-database)
for the full acceptance criteria and licensing split.

---

## Key Features

| Feature | Detail |
|---|---|
| **Pure Safe Rust** | `#![forbid(unsafe_code)]` on all crates — compiler-enforced, zero C/C++ dependencies |
| **Search engine** | A\*/AND-OR tree search (Retro\*-equivalent, pluggable `MoleculeValueEstimator`/`ReactionPrior`) with `--beam-width N` for memory-bounded exploration and `rayon` parallel rule application (sequential fallback on wasm32) |
| **Up to 50k reaction templates** | Auto-extracted from USPTO-50k/MIT via rdchiral; frequency-weighted priority (optional pure-Rust `tract-onnx` NN scorer via `--scorer`); `--templates` for custom sets |
| **Template quality tools** | `renkin template stats\|validate\|dedup\|explain\|coverage\|ids` — frequency distribution, validity, duplicates, per-template lookup, coverage rate, stable IDs |
| **Stable template IDs + evidence sidecar** | Every template gets a stable `template_id` (`rule:<name>` / `smirks-sha256:<hex>`, independent of file order). Attach curated DOIs/patents, conditions, yields, and side-reaction warnings via a `--template-metadata sidecar.json`; matching steps get an `evidence` field — see [Template evidence metadata](#template-evidence-metadata) below. `schema_version: 2` sidecars can also attach `examples` (curated exact-substrate records, surfaced first in `--format explain`). Automatic yield/success prediction and literature search remain out of scope ([#41](https://github.com/kent-tokyo/renkin/issues/41)) |
| **Ring-context safety guard** | `--ring-context-policy conservative --ring-context-sidecar <path>` — opt-in match-level filter that rejects an extracted template's ring-opening/closing disconnection when its historical training data never observed that bond as ring-forming/-breaking; default `disabled` (unchanged legacy behavior) — see [Issue #72](https://github.com/kent-tokyo/renkin/issues/72) |
| **LightGBM candidate reranker** | `--reranker-model`/`--reranker-freq-table` (CLI) or `reranker_model_path`/`reranker_freq_table_path` (Python) — opt-in, ordering-only re-ranking via a frozen LightGBM model; never changes which candidates are generated, only their order, and reproduces legacy ordering byte-for-byte when off. Paired 100-target route-search gate: `route_to_configured_stock` 16→20 (+4/-0). `python3 scripts/fetch_reranker_model.py` fetches the frozen model (SHA-256-verified, not bundled in any package — see [Roadmap](#roadmap)) |
| **Coverage mode** (opt-in) | `--search-mode coverage --coverage-templates <path>` (CLI) or `search_mode="coverage"`, `coverage_templates_path=...` (Python) — if the default template set finds no route, automatically escalates to a larger, separately loaded template set, cooperatively cancellable via `--coverage-timeout-secs`. Standard-mode output is byte-for-byte unchanged when not used. `python3 scripts/fetch_coverage_templates.py` fetches the frozen 2,000-template Stage-2 set (SHA-256-verified, not bundled in any package, same reasoning as the reranker model — see [Roadmap](#roadmap)) |
| **RENKIN Bridge / `audit-route`** | `renkin audit-route route.json [--format auto\|renkin\|aizynthfinder\|syntheseus\|synplanner] [--stock stock.smi] [--output human\|json]` — tool-neutral route audit: structural integrity, stock, and declared-reaction forward-replay validation, each reported independently as `pass`/`fail`/`not_evaluable`, rolled up into a route-level `pass`/`fail`/`partial` verdict. Reads RENKIN-native route JSON (v0.25.0), real AiZynthFinder route JSON — single-target and gzip-compressed batch output, verified against AiZynthFinder 4.3.2, 4.4.0, and 4.4.1 specifically, not claimed for every version (v0.26.0, version matrix widened v0.32.0) — Syntheseus routes via the optional `renkin.syntheseus_exporter`'s `syntheseus-route-v1` interchange schema, since Syntheseus itself has no native route export (v0.30.0) — and real SynPlanner 1.6.0 `write_routes_json` exports directly, no exporter package needed (v0.34.0); `--format auto` detects the input shape and hard-errors rather than guessing on anything ambiguous. [AiZynthFinder walkthrough →](https://kent-tokyo.github.io/renkin/guides/aizynthfinder-audit-demo/) · [Syntheseus walkthrough →](https://kent-tokyo.github.io/renkin/guides/syntheseus-audit-demo/) · [SynPlanner walkthrough →](https://kent-tokyo.github.io/renkin/guides/synplanner-audit-demo/) |
| **Route scoring** | `confidence`, `step_confidence`, `success_probability` (Retro-prob style), `convergency`, `atom_economy`, `route_cost` (`Σ BB cost + steps×0.5`, or actual prices via `--bb-prices`/`--stock`) per step/route — see caveat below the table |
| **Step metadata provenance** | Each step reports `metadata_source`/`metadata_scope` so it's machine-readable whether `conditions`/`reaction_family` came from a rule-author default vs. something more grounded; absent (not fabricated) for extracted templates |
| **Pareto multi-objective search** | `--format pareto` returns a Pareto front across `route_cost`/`success_probability`/`steps`; objectives configurable via `--objectives` |
| **Constraint DSL** | `--constraints constraints.json` — element filters, step limits, confidence thresholds, preferred reaction families; enables LLM → RENKIN pipelines |
| **Output formats & diagnostics** | `--format json\|tree\|mermaid\|explain\|compare\|compare-json\|pareto`; zero-route JSON includes a `diagnostics` block with `likely_causes`/`suggestions` |
| **`renkin-forward` toolkit** | `predict` (rank forward products), `enumerate` (bounded products from one reactant + partner library), `hints` (partner-free retrieval hints, no concrete product), `validate` (forward-verify each retro step) — see the [Forward guides](docs/guides/forward-retrieval-hints.md#predict--enumerate--hints-at-a-glance) |
| **`renkin-bench`** | USPTO-50k/PaRoutes evaluation with `--plausibility` (forward-validated composite score), `--failure-taxonomy`, atom-balance checks (`target_MW > Σ precursor_MW`), and multi-stage `cascade` re-runs on unsolved targets — see [Benchmark](#benchmark) |
| **Stock CSV management** | `renkin stock stats\|validate\|coverage` — SMILES, name, vendor, price, hazard fields |
| **MCP server** | `renkin-mcp` exposes 6 tools: `find_routes`, `validate_route`, `explain_route`, `find_pareto_routes`, `plan_with_constraints`, `estimate_diversity` |
| **`renkin-doctor`** | Environment diagnostic binary — templates, building blocks, Python import, tool versions, data integrity |
| **`renkin-kg`** | Reaction knowledge graph builder — bipartite mol↔reaction graphs from routes, GraphML/Cypher export |
| **Multi-target** | `pip install renkin` (pre-built wheels, Linux/macOS/Windows) · `npm install renkin` (~500 KB WASM, near-native browser speed) |
| **Building blocks + stereo** | 402 unique compounds loaded from `data/building_blocks.smi` (aryl halides, boronic acids, heterocycles, amines, acids, amino acids — see [Benchmark](#benchmark)); full tetrahedral @/@@ and E/Z stereochemistry; `building_blocks` field in every route JSON (leaf starting-material SMILES, no manual parsing) |

> **`step_confidence`/`success_probability` are not yields or measured success rates.**
> They're template-frequency-derived search-ranking scores (`rule_weight / max_rule_weight`,
> multiplied across a route's steps) used to order candidate disconnections during search —
> not a calibrated probability of experimental success, and not an expected isolated yield.
> Route-level experimental yield/success-rate reporting is not implemented.

---

## Pipeline Examples

```bash
# Route cost scoring with commercial prices
renkin -t "Cc1ccc(-c2ccccc2)cc1" --bb-prices data/prices.csv --format json

# Standalone forward prediction — no route search involved
renkin-forward predict --reactants "Oc1ccccc1C(=O)O" "CCO" --report --max-results 5

# Forward validation — pipe find_routes output directly
renkin -t "CC(=O)Oc1ccccc1C(=O)O" --format json | renkin-forward validate

# Faster template retrieval with bond-center index (~24% speedup)
renkin -t "c1ccc(NC(=O)c2ccccc2)cc1" --templates data/templates_extracted_5000.smi --bond-index

# Audit a real AiZynthFinder route export with RENKIN (see the full walkthrough:
# https://kent-tokyo.github.io/renkin/guides/aizynthfinder-audit-demo/)
renkin audit-route tests/fixtures/aizynthfinder/v4.4.1/single_trees.json \
  --format aizynthfinder \
  --stock data/building_blocks.smi \
  --output human

# Same audit pipeline, either source — --format auto also detects both correctly
renkin -t "CC(=O)Oc1ccccc1C(=O)O" --format json > /tmp/renkin-route.json
renkin audit-route /tmp/renkin-route.json --format renkin
renkin audit-route tests/fixtures/aizynthfinder/v4.4.1/single_trees.json --format aizynthfinder
```

---

## Benchmark

USPTO-50k test set (4,907 molecules, full evaluation):

> **Evaluation definition**: A molecule is *solved* if `find_routes` returns at least one route whose leaf precursors are all in the building block set, within depth=5 and beam=100. Ground-truth reactants from USPTO-50k are **not** checked — any commercially accessible route counts.

### Corrected baseline (commit `e20dc8c`, 2026-07-22)

| Public label | Internal metric | Value |
|---|---|---|
| Search-to-stock rate | `raw_solved_rate` | **20.09%** (986/4,907) |
| Atom-balance-filtered rate | `atom_balanced_solved_rate` | **15.41%** (756/4,907) — subset of search-to-stock |
| Current-validator-confirmed rate | `provenance_validated_solved_rate` | **0.88%** (43/4,907) — subset of atom-balance-filtered |

402 building blocks (unique compounds actually loaded from `data/building_blocks.smi` — see below), 5,000 extracted templates, 28 handcrafted rules, depth=5, beam=100. These three rates are a nested series over the same 4,907 targets, not independent numbers, and none is an experimentally-verified synthesis success rate or a human-chemist-reviewed route-accuracy figure. `provenance_validated_solved_rate` is not a measured chemical-accuracy rate and not a proven lower bound on correctness — it only counts routes the current validator can positively confirm, and an unknown fraction of "invalid" verdicts may be validator false negatives rather than real chemistry or route errors (the split is unmeasured). Full methodology, per-rule breakdown, and reproduction command: [`tasks/phase31_final_remeasurement_run.md`](https://github.com/kent-tokyo/renkin/blob/master/tasks/phase31_final_remeasurement_run.md) · [Full benchmark details →](https://kent-tokyo.github.io/renkin/benchmark/)

### Historical progression (pre-fix, invalidated — see notice above)

⚠️ The figures in this subsection (78.0% single-pass, 95.9% cascade, 81.8% ChEMBL OOD) predate the 31.11/31.12 fixes, are invalidated, and have not been re-measured. Kept for continuity only — do not cite as current performance.

> **Evaluation note**: All numbers use the standard USPTO-50k train/test split (same corpus). Templates are extracted from the training set and evaluated on the test set. Numbers reflect performance within the USPTO-50k domain; out-of-distribution generalization was separately evaluated via ChEMBL approved drugs (**81.8%**, 409/500, also not re-measured).

| Config | Solved | Rate | BBs | Templates | depth | beam | ms/mol |
|---|---|---|---|---|---|---|---|
| v0.1.0 initial | 366/4907 | 7.5% | 463 | 31 | 3 | 50 | — |
| + auto templates (top-300) | 1363/4907 | 27.8% | 463 | 222 | 3 | 50 | — |
| + depth=5, top-500 templates | 2315/4907 | 47.2% | 463 | 314 | 5 | 50 | — |
| + beam=100 | 2688/4907 | 54.8%* | 463 | 314 | 5 | 100 | — |
| + Phase A (template freq. weighting) | 3540/4907 | 72.1%† | 463 | 314 | 5 | 100 | — |
| + 5,000 templates, 480 BBs | 3826/4907 | 78.0% | 480 | 5,000 | 5 | 100 | 2,775 |
| Phase A unlimited (beam=0) | 3832/4907 | 78.1% | 480 | 5,000 | 5 | 0 | — |
| Phase B (NN scorer, tract-onnx) | 3826/4907 | 78.0% | 480 | 5,000 | 5 | 100 | 3,394 |
| **+ diaryl sulfone rule, 509 BBs** | **3826/4907** | **78.0%** | **509** | **5,000** | **5** | **100** | **≈2,800** |
| Cascade (stage2: depth=7, beam=300 on unsolved) | 4705/4907 | **95.9%** | 509 | 5,000 | 7 | 300 | — |

\* 29/50 chunks, previous binary  
† 50/50 chunks — **72.1%** (3,540/4,907) confirmed  
BB counts in this historical table (463/480/509) are as originally documented at each point in time — legacy documentation values, not re-verified against `ChemEnv::bb_count()`. The corrected-baseline section above uses the actually-loaded count (402) for the current `data/building_blocks.smi`.

*Note: LocalRetro (53.4%) and GLG (58.0%) report single-step top-1 prediction accuracy — a different metric, not directly comparable.*

> **Benchmark scope note**: USPTO-50k is used here as a *standardized sanity benchmark*, not as proof of broad real-world synthesis performance. The corpus covers a narrow slice of reaction space (primarily C–C and C–N bond formations common in pharmaceutical synthesis), and reaction types with sparse USPTO representation are systematically underserved. Out-of-distribution performance on ChEMBL approved drugs (**81.8%**, 409/500, pre-fix, not re-measured) suggested the rule set generalizes beyond the test corpus, but neither historical number should be interpreted as a guarantee of route quality on arbitrary targets.

### PaRoutes compatibility

RENKIN is compatible with the [PaRoutes](https://github.com/AstraZeneca/PaRoutes) multi-step benchmark. Download their stock compounds and target molecules, then pass them directly:

```bash
renkin-bench \
  --input paroutes_n1_targets.smi \
  --building-blocks paroutes_stock.smi \
  --templates data/templates_extracted_5000.smi \
  --depth 5 --beam-width 100
```

The JSON output includes `avg_nodes_expanded`, `avg_confidence`, `avg_convergency`, and `avg_success_prob` (Retro-prob style) alongside the standard solved/success_rate metrics.

---

## Competitive Landscape

⚠️ RENKIN's row below uses the corrected `raw_solved_rate` (20.09%, see notice near the top of this README) — the 95.9% cascade figure some earlier versions of this table cited is invalidated and not re-measured; it is not included here.

| Tool | Language | License | WASM | Zero-dep | Algorithm | Template source | Stock |
|---|---|---|---|---|---|---|---|
| **ASKCOS** | Python | CC BY-NC | No | No (Docker, 64 GB) | MCTS + A\* | USPTO (ML) | ZINC |
| **AiZynthFinder** | Python | MIT | No | No (conda + model) | MCTS | USPTO (ML, ~50k) | eMolecules (~6M) |
| **SYNTHIA** | Closed | Proprietary | No | No | SMARTS + AND/OR | Manual curated | Sigma-Aldrich |
| **IBM RXN** | Closed | Cloud SaaS | No | No | Transformer | USPTO | — |
| **Retro\*** | Python | MIT | No | No (unmaintained) | A\* + AND/OR | USPTO (ML) | eMolecules |
| **★ RENKIN** | **Rust** | **MIT** | **Yes** | **Yes** | **A\* + AND/OR** | Hand-curated + rdchiral (5k default; 50k via `--templates`) | 402+ |

`raw_solved_rate` is the closest available RENKIN metric to the published route-finding success rates of the other planners above, but the figures are not directly comparable — stock size, template library, target set, search budget, and route-quality checks all differ across systems, and this table does not establish RENKIN as better or worse than the alternatives.

**RENKIN's goal**: match state-of-the-art accuracy using only curated rules and auto-extracted SMIRKS templates — no GPU, no training data, no black boxes. Under RENKIN's benchmark setting (corrected baseline, commit `e20dc8c`, 2026-07-22), it reaches **20.09%** `raw_solved_rate` (986/4,907) single-pass — see the Benchmark section above for the full nested-metric series and why the stricter `provenance_validated_solved_rate` (0.88%) is not RENKIN's measured or bounded correctness rate. RENKIN runs anywhere: browser, CLI, Python — single `cargo build`.

> ⚠️ The table above lists tools under different evaluation conditions. No matched-condition experiment against other tools has been performed.

---

## MCP Server

`renkin-mcp` exposes retrosynthesis as an MCP tool so AI agents (Claude, etc.) can call it directly.

**Setup** — add to `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "renkin": { "command": "/path/to/renkin-mcp" }
  }
}
```

**Tools** (6):

| Tool | Description |
|---|---|
| `find_routes` | Retrosynthesis: SMILES → routes with scoring |
| `validate_route` | Forward-validate a retrosynthetic route |
| `explain_route` | Human-readable strengths/weaknesses per route |
| `find_pareto_routes` | Pareto-front multi-objective route search |
| `plan_with_constraints` | Constraint-DSL planning (element filters, step limits, confidence thresholds) |
| `estimate_diversity` | Route diversity and coverage metrics |

The server auto-detects `data/building_blocks.smi` and `data/templates_extracted_5000.smi` in the working directory. Falls back to the embedded `DEFAULT_BUILDING_BLOCKS` / `default_rules()` defaults if not found (152 unique building blocks per `ChemEnv::bb_count()`, 27 handcrafted rules — verified 2026-08-22, after `aryl_amine_retro`'s removal (issue #77) dropped the count from 28; a "509-BB / 20-rule" figure was previously documented here without verification).

```bash
cargo build --release
# binary: target/release/renkin-mcp
```

---

## Architecture

### Workspace scope

```
┌──────────────────────────────────────────────────────────────────┐
│ renkin workspace (this repository)                               │
│                                                                  │
│  renkin  (retrosynthesis)         renkin-forward                  │
│  ──────────────────────           ─────────────────────────────  │
│  target → precursors              reactants → products           │
│  A* / AND-OR search               template-based forward         │
│  route scoring & constraints      (validates retro routes)       │
│        │                                    │                    │
│        └──────────────────┬─────────────────┘                    │
│                           ▼                                      │
│               chematic  (molecular representation,               │
│               SMILES, substructure matching, reaction SMARTS)    │
└──────────────────────────────────────────────────────────────────┘
```

### Internal data flow (renkin crate)

```
Target SMILES
     │
     ▼
┌─────────────────────────┐
│     chem_env.rs         │  ← chematic wrapper
│  - SMILES parse         │     canonical-SMILES FxHashSet BB lookup (O(1))
│  - 20 built-in + up to 50k via --templates  │     fragment sanitization + ring-leak filter
│  - Building block check │     apply_retro memoization cache
└────────────┬────────────┘
             │  par_iter (rayon / sequential on WASM)
             ▼
┌─────────────────────────┐
│      search.rs          │  ← A* / AND-OR Tree Search
│  - Priority queue       │     SA Score heuristic + memoization
│  - Closed list          │     beam search (SmallVec frontier)
│  - Arc<PathNode> paths  │     O(1) path sharing per child
└────────────┬────────────┘
             │
             ▼
┌─────────────────────────┐
│      score.rs           │  ← Heuristic / Cost Function
│  - SA Score (chematic)  │     h = Σ(1 + 0.5·(sa−1)/9)
│  - MW step cost         │     g = Σ(1 + total_mw/2000)
└────────────┬────────────┘
             │
             ▼
┌─────────────────────────┐   (optional)
│      scorer.rs          │  ← Phase B: NN Template Scorer
│  - tract-onnx           │     Pure Rust ONNX inference
│  - --scorer flag        │     molecule-specific template ranking
└────────────┬────────────┘
             │
             ▼
  JSON  ←  CLI / Python / WASM
```

---

## Project Structure

```
renkin/                          ← Cargo workspace root
├── Cargo.toml
├── src/                         ← renkin crate (retrosynthesis)
│   ├── lib.rs                   # public library
│   ├── main.rs                  # CLI binary (--templates, --template-metadata, --scorer, --constraints, --objectives flags)
│   ├── bin/benchmark.rs         # renkin-bench binary (--plausibility flag)
│   ├── bin/doctor.rs            # renkin-doctor diagnostic binary
│   ├── bin/fp.rs                # renkin-fp ECFP4 fingerprint (nn-scoring feature)
│   ├── bin/mcp.rs               # renkin-mcp MCP server (6 tools)
│   ├── chem_env.rs              # retro rules + BB lookup + template loader
│   ├── score.rs                 # SA Score heuristic + step cost
│   ├── search.rs                # A* / AND-OR tree engine + beam pruning
│   ├── scorer.rs                # Phase B: tract-onnx NN template scorer
│   ├── candidate.rs             # one-step candidate proposal (offline reranking foundation, not wired into search)
│   ├── pool_export.rs           # candidate-pool JSONL + reproducibility-manifest export
│   ├── python.rs                # PyO3 bindings (--features python)
│   └── wasm.rs                  # wasm-bindgen bindings (cfg = wasm32)
├── crates/                      ← sibling crates
│   ├── renkin-forward/          # forward reaction prediction (reactants → products)
│   └── renkin-kg/               # reaction knowledge graph builder (GraphML / Cypher export)
├── data/
│   ├── building_blocks.smi              # 402 curated commercial starting materials (loaded/deduplicated count)
│   ├── templates_extracted_5000.smi     # 5,000 auto-extracted SMIRKS templates
│   ├── benchmark_targets.smi            # internal benchmark set
│   └── bench_chunks/                    # USPTO-50k per-chunk results
├── scripts/
│   ├── extract_templates.py         # rdchiral template extraction pipeline
│   ├── run_benchmark_chunks.sh      # resumable chunked benchmark runner
│   ├── train_reranker.py            # candidate reranker training/evaluation (dev tool, offline only — see docs/guides/reranker-candidate-pools.md)
│   └── tests/                       # unittest suite for train_reranker.py
├── docs/                # MkDocs source → kent-tokyo.github.io/renkin/
└── mkdocs.yml
```

---

## Roadmap

Full shipped history (every release, in order): [`CHANGELOG.md`](CHANGELOG.md).
This section only tracks the current headline items and what's next —
see "Earlier milestones" below for older shipped work.

### Recently shipped

- [x] **SynPlanner Bridge** (`--format synplanner`, shipped v0.34.0) — *Keep SynPlanner. Audit its routes with RENKIN.* A fourth route adapter, and the first whose native export carries real, forward-replayable atom mapping: `renkin audit-route --format synplanner` (also auto-detected) reads real SynPlanner 1.6.0 `write_routes_json` exports directly, no exporter package needed. Confirmed against real SynPlanner output twice — hand-constructed reactions through SynPlanner's own exporter, and a real CPU-only MCTS-searched planning run through the real `synplan planning` CLI end to end — every one of 317 real reaction nodes across a 167-route real search carries a structurally valid atom map, and real routes reach a genuine `pass` verdict, not just `not_evaluable`. New 4-way (RENKIN-native/AiZynthFinder/Syntheseus/SynPlanner) structural and policy-verdict parity tests. The [browser playground](https://kent-tokyo.github.io/renkin/playground/)'s Audit tab gained SynPlanner as a fourth format option, with a one-click real-MCTS-output example. [5-minute walkthrough with real output →](https://kent-tokyo.github.io/renkin/guides/synplanner-audit-demo/)
- [x] **Syntheseus Bridge** (`--format syntheseus`, shipped v0.30.0) — *Syntheseus has no route export. RENKIN built one — and audits it exactly like every other adapter.* A third route adapter alongside RENKIN-native and AiZynthFinder: the optional `renkin.syntheseus_exporter` (`pip install renkin[syntheseus]`) turns a real Syntheseus `SynthesisGraph` into the `syntheseus-route-v1` interchange schema, which `renkin audit-route --format syntheseus` (also auto-detected) consumes through the identical audit pipeline every adapter shares. Forward validation honestly reports `not_evaluable` for every real Syntheseus route today — `reaction_smiles` carries no atom mapping, never faked into a pass. The [browser playground](https://kent-tokyo.github.io/renkin/playground/)'s Audit tab gained Syntheseus as a third format option. [5-minute walkthrough with real output →](https://kent-tokyo.github.io/renkin/guides/syntheseus-audit-demo/)
- [x] **Audit Policy Profiles** (`--policy informational|standard|strict`, shipped v0.29.0) — *One set of findings. Three ways to derive the verdict.* Audit the same route under `informational`, `standard`, or `strict` policy without ever hiding or changing the underlying findings — policy only changes how the overall pass/fail/partial verdict is derived from findings already collected, recorded in `audit_manifest.policy`. Consistent across every surface: `renkin audit-route --policy`, the Rust API, the first Python binding for route auditing (`renkin.audit_route()`), and a new WASM `audit_route_v2()` (the existing `audit_route()` stays as a `standard`-policy wrapper). The [browser playground](https://kent-tokyo.github.io/renkin/playground/)'s Audit tab gained a policy selector.
- [x] **Audit Playground** (`[ Audit a Route ]` tab, shipped v0.28.0) — *Audit a route in your browser — the same pipeline, the same verdict, zero network calls.* The [browser playground](https://kent-tokyo.github.io/renkin/playground/) now audits a RENKIN or AiZynthFinder route export (single-route or Pandas batch) and an optional stock list entirely client-side, via a new `audit_route` WASM export that calls the identical report-building pipeline `renkin audit-route` uses — the same pass/fail/partial verdict either way, not a separately-maintained copy. Paste or upload, run off the main thread, download the JSON report.
- [x] **Reproducible Route Audit** (`audit_manifest` on `renkin audit-route --output json`, shipped v0.27.0) — *Reproduce what was audited, from which input, with which stock and policy.* Every audit report now records RENKIN version, report schema version, source format/version, input/stock content SHA-256 hashes, and audit policy — tested for byte-identical determinism (auditing the same input twice), not just claimed. Adds a shared adapter conformance suite across RENKIN-native and AiZynthFinder route inputs, plus a written [reproducibility/compatibility contract](https://kent-tokyo.github.io/renkin/guides/audit-reproducibility-contract/) (verified-vs-supported versions, unknown-field tolerance, report-schema rules, adapter-fixture runbook). The [browser playground](https://kent-tokyo.github.io/renkin/playground/) also got a safety/UX pass this release: search runs off the main thread with cancel/time-budget support, structure rendering stays local by default (no third-party SMILES transmission), and search settings round-trip exactly through Copy CLI/Python.
- [x] **RENKIN Bridge — Cross-Tool Route Audit** (`renkin audit-route`, RENKIN-native adapter shipped v0.25.0, AiZynthFinder adapter shipped v0.26.0) — *Keep AiZynthFinder. Audit its routes with RENKIN.* A tool-neutral route audit model: structural-integrity, stock, and declared-reaction forward-replay validation, each reported independently as `pass`/`fail`/`not_evaluable`, rolled up into a route-level `pass`/`fail`/`partial` verdict — never a silently force-passed boolean. v0.26.0 adds a real AiZynthFinder adapter (single-target and gzip batch JSON, verified against captured v4.4.1 output — see [`PROVENANCE.md`](tests/fixtures/aizynthfinder/v4.4.1/PROVENANCE.md)) plus `--format auto` detection, so both tools' routes run through the exact same audit pipeline; auditing the real fixtures also surfaced and fixed a shared forward-replay bug where precursor ordering, not just chemistry, affected the verdict. `renkin audit-route route.json --stock stock.smi --output json` audits every route in a file and aggregates the results into one machine-readable report, regardless of which tool produced it.
- [x] Coverage mode (`--search-mode coverage`, [#101](https://github.com/kent-tokyo/renkin/issues/101), shipped v0.24.0) — opt-in Stage-1/Stage-2 template-count escalation, addressing the candidate-generation coverage gap below. Confirmed by a one-shot 500-target formal-TEST (`data/coverage_mode_formal_test/protocol_v2.md`): coverage +6.0pp, net gain +30, zero regressions, zero reranker failures, Stage-2 timeout rate 0.25% — all against pre-registered thresholds. See the Key Features table above for the shipped surface
- [x] Reranker made actually usable: Python exposure (`find_routes()`'s `reranker_model_path`/`reranker_freq_table_path`) and batteries-included model distribution (`scripts/fetch_reranker_model.py`, SHA-256-verified fetch from the v0.22.0 GitHub Release's canonical assets) ([#101](https://github.com/kent-tokyo/renkin/issues/101), shipped v0.23.0) — v0.22.0 proved the reranker works; v0.23.0 is the usability/distribution unlock, not a new accuracy claim
- [x] LightGBM candidate reranker, trained/gated offline and wired into route search ([#101](https://github.com/kent-tokyo/renkin/issues/101) Task 35, CLI shipped v0.22.0) — LambdaMART model trained on real USPTO-50k labels, passed its VAL screening gate (top1 +11.7pp, MRR +11.3pp, top10 +9.3pp, bootstrap-CI-confirmed) and a formal 4,903-target TEST evaluation against the frozen model exactly once (top1 +12.7pp, MRR +11.9pp, top10 +9.1pp — consistent magnitude with VAL, no overfitting signal), then wired into `find_routes` as an ordering-only rank bonus and confirmed with a paired 100-target route-search gate: `route_to_configured_stock` 16→20/100 (+4/-0). See the Key Features table above
- [x] Formal 500-target RENKIN vs AiZynthFinder comparison ([#66](https://github.com/kent-tokyo/renkin/issues/66)) — under a fixed 500-target sample, shared 393-compound stock, and each tool's configured policy/budget, RENKIN Conservative's `route_to_shared_stock` outcome was 9.8 percentage points higher than AiZynthFinder's (73/500 vs 24/500, 95% CI [7.0, 12.8], exact McNemar p≈1.9e-11) — a statistically significant paired difference under this protocol, not a general search-capability superiority claim. Native-mode configurations (each tool's own stock) diverge in the opposite direction, dominated by unmatched conditions including a large stock-size gap. See the [comparison guide](docs/guides/open-source-retrosynthesis-comparison.md) for the full, deliberately scoped interpretation.
- [x] Ring-context safety guard for extracted templates ([#72](https://github.com/kent-tokyo/renkin/issues/72)/[#242](https://github.com/kent-tokyo/renkin/pull/242)) — opt-in `--ring-context-policy`/`--ring-context-sidecar`, catches extracted templates silently misapplying a ring-opening/closing disconnection their training data never saw; default remains `disabled` (unchanged legacy behavior)
- [x] `atom_economy` no longer silently clamped to 100% when a route's represented precursor set can't account for the target's full mass ([#79](https://github.com/kent-tokyo/renkin/issues/79)) — a new `atom_economy_status` field (`normal`/`above_expected_range`/`not_evaluable`) reports this explicitly instead

### In progress

- [ ] Candidate-generation coverage gap — 33.0% (1,618/4,903) of the formal TEST corpus has zero positive candidates in-pool, a ceiling reranking cannot fix by construction; template-diversity-scaling confirmed as a strong mechanism (Phase A.5/B.2, see coverage mode above), higher-level-template research direction not yet started
- [ ] Template retrieval index (element bitmask + bond-center prefilter) for the 50k template set
- [ ] Calibrated route confidence (map `success_probability` to empirical solve rate)

### Next

- [ ] Graph rule expansion — sulfonamide / carbamate / urea cleavage (one PR per family, each with benchmark delta)
- [ ] Stock-aware planning (price / hazard / availability re-ranking)

<details>
<summary>Earlier milestones</summary>

Percentage figures below are historical milestones at the time each was
shipped, not current performance — several predate the validator-accuracy
fix noted in [Current Limitations](#current-limitations) and are invalidated;
see [Benchmark](#benchmark) for the corrected historical baseline.

- [x] Reranker made actually usable: Python exposure + batteries-included model distribution ([#101](https://github.com/kent-tokyo/renkin/issues/101), v0.23.0) — see "Recently shipped" above for the current-cycle summary; full detail in [`CHANGELOG.md`](CHANGELOG.md)
- [x] Stable `template_id` (`rule:<name>` / `smirks-sha256:<hex>`) + `--template-metadata` evidence sidecar + `renkin template ids` ([#41](https://github.com/kent-tokyo/renkin/issues/41) phase 1)
- [x] Substrate-specific `examples` (`schema_version: 2`) — per-step exact-substrate vs. same-template-different-substrate resolution, surfaced in `--format explain` and as `match_kind` in JSON ([#41](https://github.com/kent-tokyo/renkin/issues/41) phase 2)
- [x] Deterministic ORD (Open Reaction Database) evidence import — offline `renkin evidence match` exact-set batch template matcher + `scripts/ord_evidence_audit.py` audit/converter into `schema_version: 2` sidecars ([#41](https://github.com/kent-tokyo/renkin/issues/41) phase 3A)
- [x] RETROSPECT-inspired offline candidate-reranking foundation — candidate proposal/selection separation, feature schema v1, manifest v2, leakage-safe train/val/test splitting, baseline arms + trained-ranker arm, paired bootstrap + offline gate tooling ([#59](https://github.com/kent-tokyo/renkin/pull/59))
- [x] `renkin-forward enumerate` — bounded, template-guided forward enumeration from a single known reactant plus an explicit partner library ([#64](https://github.com/kent-tokyo/renkin/issues/64))
- [x] `renkin-forward hints` — partner-free retrieval hints (matched template slots, missing-partner SMARTS, bond deltas) for patent/database search, no concrete product predicted ([#64](https://github.com/kent-tokyo/renkin/issues/64) phase 2)
- [x] `renkin-forward` CLI hardening — versioned `ForwardPredictionReport`, deterministic candidate IDs/merge/provenance, reactant-order-independent matching, strict CLI/route-JSON validation
- [x] `apply_retro`/`run_reactants` performance regression resolved — `chematic` moved to the published `0.8.0` release (upstream automorphism-orbit-pruned canonicalization, [chematic#193](https://github.com/kent-tokyo/chematic/pull/193)); zero correctness change
- [x] `renkin-bench cascade` — multi-stage search (fast defaults → hard cases re-run deeper); only unsolved targets propagate to later stages
- [x] `renkin-bench --failure-taxonomy` — classify unsolved targets by cause (beam limit / depth limit / template gap / stock near-miss)
- [x] Graph-based ester cleavage — BFS-leakage-free `R-C(=O)-O-R' → RCOOH + R'OH`
- [x] `--top-templates N` — frequency-rank filter: use the top-N most frequent templates for speed / less noise
- [x] `raw / validated / practical` solved-rate metrics (`--plausibility --practical-max-steps N`)
- [x] Retro cache hit-rate in `SearchStats` + `--verbose`
- [x] Route cost scoring — `route_cost` field + `--bb-prices path.csv` / `--stock stock.csv`
- [x] Cargo workspace — `crates/renkin-forward/` + `crates/renkin-kg/`
- [x] `renkin-forward predict` / `validate` — forward prediction + route validation (stdin-pipe friendly)
- [x] `renkin-doctor` — environment diagnostic binary (templates, BBs, Python, binaries)
- [x] Failure diagnostics — zero-route output includes `likely_causes` + `suggestions` JSON block
- [x] `--format explain|compare|compare-json` — human-readable and tabular route output
- [x] `renkin stock stats|validate|coverage` — stock CSV management subcommand
- [x] Pareto multi-objective search — `--format pareto`, `--objectives`, `find_pareto_routes` MCP
- [x] Constraint DSL — `--constraints JSON`, `plan_with_constraints` MCP tool
- [x] `renkin template stats|validate|dedup|explain|coverage` — template quality tools
- [x] `renkin-kg` — reaction knowledge graph (bipartite mol↔reaction, GraphML/Cypher export)
- [x] MCP server (`renkin-mcp`) — expanded to 6 tools (`explain_route`, `find_pareto_routes`, `plan_with_constraints`, ...)
- [x] Core search engine foundation — SMIRKS retro-reaction rules + fragment sanitization, A\*/AND-OR tree search with closed list + degenerate-route filter, SA Score heuristic + beam search, `rayon` parallel rule application (sequential fallback on WASM), FxHashMap/SmallVec beam frontier/SA-Score-memoization/`Arc<PathNode>` path-sharing perf work
- [x] Multi-target packaging — Python bindings (PyO3 + maturin, `pip install renkin`), WASM build (`npm install renkin`), published to crates.io/PyPI/npm with GitHub Actions CI/CD, WASM browser playground + i18n (EN/JA/ZH)
- [x] Benchmark CLI (`renkin-bench`) + USPTO-50k evaluation, `--format tree|mermaid` visualization, MkDocs documentation site + GitHub Pages playground
- [x] Graph-based biaryl cleavage · O(1) canonical-SMILES BB index
- [x] Tetrahedral stereo @/@@ + E/Z double-bond stereo
- [x] NN template scorer via `--scorer` flag (tract-onnx, Pure Rust ONNX)
- [x] Constraint-based search (`--avoid-elements`, `--require-elements`) + `--verbose` search statistics
- [x] `#![forbid(unsafe_code)]` — compiler-enforced Pure Safe Rust from the start

</details>

---

## Citation

If you use RENKIN in academic work, please cite it — see [`CITATION.cff`](CITATION.cff)
for the canonical, version-tracked citation record. GitHub's "Cite this
repository" button (top of the repo page) reads it directly and can export
APA or BibTeX on demand.

---

## Security

Report vulnerabilities via [GitHub Private vulnerability reporting](https://github.com/kent-tokyo/renkin/security/advisories/new). See [SECURITY.md](SECURITY.md).

---

## License

MIT

---

*GitHub Topics: `retrosynthesis` `cheminformatics` `wasm` `rust` `drug-discovery` `casp` `synthesis-planning` `computational-chemistry`*

---

If RENKIN saves you time, a GitHub star helps others discover it.
