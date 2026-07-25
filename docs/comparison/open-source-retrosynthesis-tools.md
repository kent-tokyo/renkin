---
title: "RENKIN vs AiZynthFinder, ASKCOS and Syntheseus: Open-Source Retrosynthesis Tools"
description: "A feature comparison of open-source retrosynthesis/CASP tools -- RENKIN, AiZynthFinder, ASKCOS, and Syntheseus -- by language, dependencies, deployment, and capabilities, not performance ranking."
---

# Open-Source Retrosynthesis Tools: A Feature Comparison

This page compares RENKIN against three other open-source computer-aided
synthesis planning (CASP) projects on **verifiable, structural differences**
— language, dependencies, deployment model, and feature set. It does not rank
success rate or speed: these tools use different stock databases, template
sets, search budgets, and evaluation methodologies, so a head-to-head
performance number would not be a fair comparison without a matched-condition
experiment, which none of these projects (including RENKIN) has published.
See RENKIN's own [Benchmark page](../benchmark.md) for what limited, heavily
caveated performance data does exist for RENKIN specifically.

## At a Glance

| | [RENKIN](https://github.com/kent-tokyo/renkin) | [AiZynthFinder](https://github.com/MolecularAI/aizynthfinder) | [ASKCOS v2](https://gitlab.com/mlpds_mit/askcosv2/askcos2_core) | [Syntheseus](https://github.com/microsoft/syntheseus) |
|---|---|---|---|---|
| Maintainer | Independent | AstraZeneca (MolecularAI) | MIT (mlpds_mit consortium) | Microsoft Research |
| Core language | Rust | Python | Python (multi-service) | Python |
| License (code) | MIT | MIT | MIT (current v2; the archived [v1 GitHub repo](https://github.com/ASKCOS/ASKCOS) was MPL 2.0, with data/models under CC BY-NC-SA) | MIT |
| Install | `pip` / `cargo` / `npm`, single binary or wheel | `pip install aizynthfinder[all]` | Docker Compose or Kubernetes; no simple pip install | `pip install "syntheseus[all]"` |
| Runs in a browser (WASM) | Yes — a full client-side build | No | No | No |
| Chemistry backend | [`chematic`](https://docs.rs/chematic/) (pure Rust, no C/C++) | RDKit | RDKit + trained models | Depends on the wrapped model(s) |
| Ships its own retrosynthesis engine | Yes — rules + search built in | Yes — trained expansion policy + MCTS | Yes — multiple built-in template-based and ML models | No — orchestrates/benchmarks external models (LocalRetro, MEGAN, Chemformer, RootAligned, ...) |
| Search algorithm | A\* / beam search | Monte Carlo Tree Search over a trained policy | Multiple (template-based + template-free/Transformer) | Pluggable — implements common search algorithms over whichever model you plug in |
| Custom reaction templates | Yes (`--templates`, up to 50k SMIRKS) | Yes (trainable expansion policy + custom stock) | Yes, via self-hosted configuration | Depends on the wrapped model |
| Curated per-template evidence (DOI/patent/yield/conditions) | Yes — native `--template-metadata` sidecar; user-supplied, no bundled evidence corpus | No native template-ID evidence sidecar documented in the current official docs | No native template-ID evidence sidecar documented in the current official docs | No native template-ID evidence sidecar documented in the current official docs |
| Minimum local footprint | Single binary, no network calls | Local Python process | Per current [ASKCOS v2 deployment docs](https://askcos-docs.mit.edu/): 4+ CPU cores, 32 GB+ RAM, x86-only (no Apple Silicon) for self-hosting | Local Python process (plus whatever the wrapped model needs, e.g. PyTorch/GPU) |

Facts above were checked directly against each project's own repository and
docs (linked in the table) as of this page's last update; ASKCOS in
particular has migrated from the archived `ASKCOS/ASKCOS` GitHub repo (v1,
no longer updated per its own README) to `askcos2_core` on GitLab (v2, the
row above) — if you land on the old repo, follow its own link to the current
one. If anything else has changed since, each project's own README/docs are
the source of truth.

## What Each Tool Is Actually For

These aren't four competing implementations of the same idea — they solve
different problems:

- **RENKIN** is a small, embeddable engine: a single Rust crate compiled to a
  CLI binary, a Python wheel, a Rust library, or a WASM module that runs
  entirely in a browser tab with no server. If you want retrosynthesis search
  inside another tool, a browser demo, an MCP-connected AI agent, or a
  resource-constrained environment, this is the shape that fits.
- **AiZynthFinder** is a planning tool built around trained neural expansion
  policies and Monte Carlo Tree Search, developed at AstraZeneca (MolecularAI)
  and published as open source. It's the closest architectural peer to RENKIN
  in scope (a standalone library you run locally), but it's Python/RDKit-based
  and its search relies on trained models rather than (only) hand-curated or
  extracted rules.
- **ASKCOS** is a full synthesis-planning *platform*, not a library — current
  ASKCOS v2 is designed to be self-hosted as a multi-service application
  (Docker Compose or Kubernetes) with template-based and template-free
  forward/retro models, a reaction-condition recommender, and more. If you
  want an organization-wide deployed service rather than an embeddable
  engine, this is that shape — at the cost of a much heavier install (4+
  cores, 32 GB+ RAM, x86-only, per its own deployment docs).
- **Syntheseus** isn't a standalone retrosynthesis engine at all — it's a
  benchmarking/orchestration framework from Microsoft Research that wraps
  *other* published models (LocalRetro, MEGAN, Chemformer, RootAligned, and
  others) behind one common interface, so you can swap models and search
  algorithms and compare them fairly. If you're doing retrosynthesis-model
  *research* rather than looking for a planning engine to embed, this is the
  tool built for that.

## Where RENKIN Is a Better Fit

- You need the engine to run **without a Python/Docker runtime** — in a
  browser, a Rust service, or a CLI tool distributed as a single binary.
- You want to attach **curated, citable evidence you supply** (a DOI, a
  reported yield, a known side reaction) to specific templates, not just a
  bare disconnection — see [Reaction Evidence Metadata](../guides/reaction-evidence.md).
  RENKIN doesn't ship a bundled evidence corpus either; the sidecar is a
  mechanism for evidence you curate yourself.
- You want **zero C/C++ dependencies** and a `cargo build`/`pip install`
  install story with no GPU, no Docker, and no multi-service deployment.

## Where Another Tool Is a Better Fit

- You need trained-model-driven route ranking (expansion policy + MCTS) and
  are comfortable with a Python/RDKit dependency stack → **AiZynthFinder**.
- You're deploying a shared, organization-wide synthesis-planning service with
  forward prediction, condition recommendation, and more, and can dedicate the
  infrastructure to it → **ASKCOS**.
- You're doing retrosynthesis-model research and need to benchmark multiple
  published models under one harness → **Syntheseus**.

## Next Steps

- [Python retrosynthesis guide](../guides/python-retrosynthesis.md) / [Rust retrosynthesis guide](../guides/rust-retrosynthesis.md)
- [Benchmark](../benchmark.md) — RENKIN's own USPTO-50k results and their current limitations
