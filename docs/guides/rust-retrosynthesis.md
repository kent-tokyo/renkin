---
title: "Rust Retrosynthesis Engine: Computer-Aided Synthesis Planning in Pure Rust"
description: "RENKIN is a computer-aided synthesis planning (CASP) engine written entirely in Rust -- zero C/C++ dependencies, embeddable as a library, CLI, or WebAssembly module."
---

# Rust Retrosynthesis with RENKIN

Most open-source retrosynthesis tools are Python packages built around RDKit
and, often, a trained neural network. RENKIN takes a different approach: the
whole engine — SMILES/SMARTS parsing, molecule canonicalization, retrosynthetic
rule application, and A\* search — is a single Rust crate with zero C/C++
dependencies.

## Install

```toml
[dependencies]
renkin = "0.21"
```

or

```bash
cargo add renkin
```

## Why Rust

- **No RDKit, no Boost, no C/C++ toolchain.** Chemistry parsing/canonicalization
  comes from [`chematic`](https://docs.rs/chematic/), a pure-Rust
  cheminformatics crate. `cargo build` is the entire build story.
- **`#![forbid(unsafe_code)]`** on every crate in the workspace — compiler-enforced,
  not just a style guideline.
- **One codebase, four targets.** The same core search code compiles to a
  native CLI binary, a Rust library (`cargo add renkin`), a Python extension
  module (PyO3), and a WebAssembly module (`wasm-bindgen`) that runs entirely
  client-side in a browser — see [WASM API](../api/wasm.md) and the [live
  playground](../playground/index.html).

## A Working Example

```rust
--8<-- "examples/quickstart.rs"
```

`find_routes` returns `Result<(Vec<Route>, SearchStats)>` — a tuple, not a
bare `Vec<Route>` — and `Route`/`ReactionStep` fields like `depth` are plain
struct fields, not methods. Full signature and types:
[Rust API reference](../api/rust.md).

## Search Algorithm

RENKIN searches with A\* / beam search over a set of retrosynthetic rules:

- 28 hand-crafted, graph-based or SMIRKS-based rules covering common
  pharmaceutical disconnections (esters, amides, Suzuki, Buchwald-Hartwig,
  Wittig, sulfonamides, and more).
- Up to 50k additional SMIRKS templates auto-extracted from USPTO-50k/MIT via
  rdchiral, loaded from a `.smi` file and weighted by training-set frequency
  (`step_cost` is discounted for high-frequency templates).
- Every template — hand-crafted or extracted — has a stable `template_id`,
  independent of file order or extraction run, so external evidence (a DOI, a
  reported yield) can be durably attached to it — see [Reaction Evidence
  Metadata](reaction-evidence.md).

`h(molecule)` for the A\* heuristic defaults to an SA-Score-based estimate and
is pluggable via the `MoleculeValueEstimator` trait; template ranking is
pluggable via the `ReactionPrior` trait.

## WASM: Running in the Browser

Because the whole engine is Rust with no OS/filesystem dependency in its
search core, it compiles directly to WebAssembly:

```bash
wasm-pack build --target web --no-default-features
```

The [live playground](../playground/index.html) is this exact build running
client-side — no server, no network call, no Python runtime. This is a real
architectural difference from Python-based CASP tools built on RDKit/PyTorch,
which can't currently compile to WASM at all.

## When to Reach for the Rust API Directly

If you're calling RENKIN from a Rust service, embedding it in another tool, or
need to avoid a Python/Docker runtime entirely (e.g. a CLI tool, a WASM
module, or a resource-constrained deployment), the Rust API is the native
surface — the Python and WASM bindings are thin wrappers over the same
`find_routes` function documented here. If you're prototyping in a Python/Jupyter
workflow instead, see the [Python retrosynthesis guide](python-retrosynthesis.md).

## Next Steps

- [Rust API reference](../api/rust.md) — `ChemEnv`, `SearchConfig`, `RetroRule`, feature flags
- [Reaction Evidence Metadata](reaction-evidence.md) — attaching conditions/yields/references to templates
- [WASM API](../api/wasm.md) — the browser/bundler entry point
