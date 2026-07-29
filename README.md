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

## What is RENKIN?

RENKIN is an open-source **retrosynthesis engine** for **computer-aided synthesis planning (CASP)** that automatically discovers optimal chemical reaction routes from a target molecule back to cheap, commercially available starting materials.

Built entirely in Rust with the [`chematic`](https://docs.rs/chematic/) cheminformatics crate — zero C/C++ dependencies, `#![forbid(unsafe_code)]` throughout. One codebase compiles to a native CLI, a Rust library, Python wheels (PyO3), and a WebAssembly module that runs entirely client-side in the browser.

---

## Installation

```bash
pip install renkin          # Python
cargo add renkin            # Rust
npm install renkin          # JavaScript / Node.js
```

---

## Live Playground

**[→ Try it now](https://kent-tokyo.github.io/renkin/playground/)** — runs entirely in WebAssembly: no installation, no server, no network calls.

---

## Quick Start

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
[Benchmark](https://kent-tokyo.github.io/renkin/benchmark/) for the current
corrected numbers, full methodology, and known limitations).

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
| **A\* / AND-OR Tree Search** | Retro\*-equivalent algorithm with pluggable heuristics (`MoleculeValueEstimator`, `ReactionPrior`) |
| **Up to 50k reaction templates** | Auto-extracted from USPTO-50k/MIT via rdchiral; frequency-weighted priority; `--templates` for custom sets |
| **Route scoring** | `confidence`, `step_confidence`, `success_probability` (Retro-prob style), `convergency`, `atom_economy` per step — see caveat below the table |
| **Step metadata provenance** | Each step reports `metadata_source`/`metadata_scope` (e.g. `handcrafted_default`/`reaction_family`) so it's machine-readable whether `conditions`/`reaction_family` came from a rule-author default vs. something more grounded; absent for extracted templates, since nothing is fabricated for them. |
| **Stable template IDs + evidence sidecar** | Every template gets a stable `template_id` — `rule:<name>` for hand-crafted rules, `smirks-sha256:<hex>` for extracted templates (independent of file order/position/count). Attach curated DOIs/patents, reported conditions, reported yields, and known side-reaction warnings via a `--template-metadata sidecar.json` file keyed by `template_id`; matching steps get an `evidence` field, everything else stays untouched — see [Template evidence metadata](#template-evidence-metadata) below. `schema_version: 2` sidecars can additionally attach `examples` — curated records tied to one exact target/precursor set, matched by canonical SMILES and surfaced first in `--format explain`. Run `renkin template ids <file.smi>` to list stable IDs for authoring a sidecar. Automatic yield/success prediction and literature search remain out of scope ([#41](https://github.com/kent-tokyo/renkin/issues/41)). |
| **Route cost scoring** | `route_cost = Σ(BB cost) + steps×0.5`; actual prices via `--bb-prices CSV` or `--stock stock.csv` |
| **Pareto multi-objective search** | `--format pareto` returns a Pareto front across `route_cost`, `success_probability`, `steps`, etc.; objectives configurable via `--objectives cost:min,success_probability:max,steps:min` |
| **Constraint DSL** | `--constraints constraints.json` — JSON-driven synthesis planning: element filters, step limits, confidence thresholds, preferred reaction families; enables LLM → RENKIN pipeline |
| **Output formats** | `--format json` · `tree` · `mermaid` · `explain` (human-readable per-route analysis) · `compare` (side-by-side table) · `compare-json` · `pareto` |
| **Failure diagnostics** | Zero-route JSON output includes `diagnostics` block with `likely_causes` and `suggestions` |
| **Standalone forward prediction** | `renkin-forward predict --reactants <SMILES>...` enumerates and ranks forward reaction product candidates from reversed SMIRKS templates, independent of route search — see the [Forward Prediction guide](docs/guides/forward-prediction.md) |
| **Forward validation** | `renkin-forward validate` verifies each step by applying templates forward; accepts `--route-json` or stdin |
| **Plausibility report** | `renkin-bench --plausibility` — forward-validates best routes and reports composite plausibility score |
| **PaRoutes benchmark** | `renkin-bench --input-format paroutes` for multi-step ground-truth evaluation with `depth_delta` and `route_diversity` |
| **Atom balance check** | `renkin-bench` flags steps where `target_MW > Σ precursor_MW` (CompleteRXN reference) |
| **Stock CSV management** | `renkin stock stats\|validate\|coverage` — inspect and validate stock CSV files with SMILES, name, vendor, price, hazard fields |
| **Template quality tools** | `renkin template stats\|validate\|dedup\|explain\|coverage\|ids` — inspect SMIRKS template sets: frequency distribution, validity, duplicates, per-template lookup, coverage rate, stable template IDs |
| **MCP server** | `renkin-mcp` exposes 6 tools: `find_routes`, `validate_route`, `explain_route`, `find_pareto_routes`, `plan_with_constraints`, `estimate_diversity` |
| **`renkin-doctor`** | Environment diagnostic binary — checks templates, building blocks, Python import, tool versions, and data integrity |
| **`renkin-kg`** | Reaction knowledge graph builder — constructs bipartite mol↔reaction graphs from routes; exports to GraphML or Cypher |
| **Beam search** | `--beam-width N` for memory-bounded exploration; `SmallVec<[FEntry; 6]>` stack-allocated frontier |
| **Parallel rule application** | `rayon` on non-WASM; sequential fallback on wasm32 |
| **tract-onnx NN scorer** | Pure Rust ONNX inference (no C++ dep) — optional `--scorer` flag for Phase B template relevance scoring |
| **`building_blocks` in JSON** | Each route includes the leaf starting-material SMILES — no manual step parsing needed |
| **Tetrahedral stereo @/@@** | Full stereochemistry support via chematic 0.4.16 |
| **Python** | `pip install renkin` — pre-built wheels for Linux/macOS/Windows |
| **WASM** | ~500 KB bundle — runs in the browser at near-native speed |
| **402 building blocks** | Aryl halides, boronic acids, heterocycles, amines, acids, amino acids (`data/building_blocks.smi`, unique compounds actually loaded — see Benchmark section) |

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

The server auto-detects `data/building_blocks.smi` and `data/templates_extracted_5000.smi` in the working directory. Falls back to the embedded `DEFAULT_BUILDING_BLOCKS` / `default_rules()` defaults if not found (152 unique building blocks per `ChemEnv::bb_count()`, 28 handcrafted rules — verified 2026-07-22; a "509-BB / 20-rule" figure was previously documented here without verification).

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

### Recently shipped

- [x] `apply_retro`/`run_reactants` performance regression resolved — `chematic` moved from a narrow git-pinned fix to the published `0.8.0` release (upstream automorphism-orbit-pruned canonicalization, [chematic#193](https://github.com/kent-tokyo/chematic/pull/193)); on a fixed 30-target gate, measured in one session against current master: total elapsed **34.7%** faster, p95 **33.8%** faster, and the single worst-case target **42.2%** faster (confirmed via repeated isolated measurement, not a one-off run). Zero correctness change (`apply_retro` call counts identical across versions)
- [x] `renkin-forward` CLI hardening — versioned `ForwardPredictionReport`, deterministic candidate IDs/merge/provenance, reactant-order-independent matching (up to 3 reactants), strict CLI/route-JSON validation
- [x] RETROSPECT-inspired offline candidate-reranking foundation — candidate proposal/selection separation, feature schema v1, manifest v2, leakage-safe train/val/test splitting, 7 deterministic baseline arms + trained-ranker arm, paired bootstrap + offline gate tooling ([#59](https://github.com/kent-tokyo/renkin/pull/59); **foundation only — no trained model or accuracy result yet, not wired into route search**)
- [x] Stable `template_id` (`rule:<name>` / `smirks-sha256:<hex>`) + `--template-metadata` evidence sidecar + `renkin template ids` ([#41](https://github.com/kent-tokyo/renkin/issues/41) phase 1)
- [x] Substrate-specific `examples` (`schema_version: 2`) — per-step exact-substrate vs. same-template-different-substrate resolution, surfaced in `--format explain` and as `match_kind` in JSON ([#41](https://github.com/kent-tokyo/renkin/issues/41) phase 2)
- [x] Deterministic ORD (Open Reaction Database) evidence import — offline `renkin evidence match` exact-set batch template matcher + `scripts/ord_evidence_audit.py` audit/converter into `schema_version: 2` sidecars; no network access, no fuzzy matching, ambiguous/unprovenanced records excluded and counted in an audit report rather than guessed at ([#41](https://github.com/kent-tokyo/renkin/issues/41) phase 3A)
- [x] `renkin-bench cascade` — multi-stage search (fast defaults → hard cases re-run deeper); only unsolved targets propagate to later stages. **78.0% → 95.9%** on USPTO-50k
- [x] `renkin-bench --failure-taxonomy` — classify unsolved targets by cause (beam limit / depth limit / template gap / stock near-miss)
- [x] Graph-based ester cleavage — BFS-leakage-free `R-C(=O)-O-R' → RCOOH + R'OH`
- [x] `--top-templates N` — frequency-rank filter: use the top-N most frequent templates for speed / less noise
- [x] `raw / validated / practical` solved-rate metrics (`--plausibility --practical-max-steps N`)
- [x] Retro cache hit-rate in `SearchStats` + `--verbose`

### In progress

- [ ] Template retrieval index (element bitmask + bond-center prefilter) for the 50k template set
- [ ] Calibrated route confidence (map `success_probability` to empirical solve rate)

### Next

- [ ] Graph rule expansion — sulfonamide / carbamate / urea cleavage (one PR per family, each with benchmark delta)
- [ ] Stock-aware planning (price / hazard / availability re-ranking)

<details>
<summary>Earlier milestones</summary>

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
- [x] MCP server expanded to 6 tools (`explain_route`, `find_pareto_routes`, `plan_with_constraints`)
- [x] SMIRKS retro-reaction rules + fragment sanitization
- [x] A\* / AND-OR tree search, closed list, degenerate-route filter
- [x] SA Score heuristic + beam search
- [x] Parallel rule application (rayon; sequential fallback on WASM)
- [x] Python bindings (PyO3 + maturin) · `pip install renkin`
- [x] WASM build · `npm install renkin`
- [x] Benchmark CLI (`renkin-bench`) + USPTO-50k evaluation
- [x] WASM browser playground + i18n (EN/JA/ZH)
- [x] Graph-based biaryl cleavage · O(1) canonical-SMILES BB index
- [x] Published to crates.io / PyPI / npm · GitHub Actions CI/CD
- [x] MkDocs documentation site · GitHub Pages playground
- [x] Auto template extraction (rdchiral): **27.8%** → **78.0%** USPTO-50k
- [x] Tetrahedral stereo @/@@ + E/Z double-bond stereo
- [x] Template frequency weighting (Phase A): **72.1%** USPTO-50k
- [x] FxHashMap · SmallVec beam frontier · SA Score memoization · Arc<PathNode> path sharing
- [x] 5,000 extracted templates + 509 BBs: **78.0%** USPTO-50k (3,826/4,907 ✅)
- [x] NN template scorer via `--scorer` flag (tract-onnx, Pure Rust ONNX)
- [x] `--format tree|mermaid` route visualization
- [x] Constraint-based search: `--avoid-elements`, `--require-elements`
- [x] `--verbose` search statistics to stderr
- [x] MCP server (`renkin-mcp`) — AI agents call retrosynthesis directly
- [x] `#![forbid(unsafe_code)]` — compiler-enforced Pure Safe Rust

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
