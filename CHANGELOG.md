# Changelog

All notable changes to RENKIN are documented in this file.  
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).  
RENKIN adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

### Added
- **6 new SMIRKS retro-rules** (total: 20 rules, was 14)
  - `aryl_chloride_retro` — Ar-Cl → Ar-H (retro-SNAr / Pd C-Cl activation)
  - `aryl_iodide_retro` — Ar-I → Ar-H (retro-Pd/Cu C-I)
  - `aryl_fluoride_snAr_retro` — Ar-F → Ar-H (retro-SNAr; F is best leaving group)
  - `aryl_chloride_to_bromide` — Ar-Cl → Ar-Br (halogen exchange retro)
  - `heck_retro` — Ar-CH=CH-R → Ar-Br + vinyl (retro-Heck)
  - `negishi_retro` — Ar-CH₂ → Ar-Br + alkyl (retro-Negishi)
- **200+ new building blocks** (total: 480+, was 277)
  - Aryl/heteroaryl bromides and chlorides (Suzuki / Buchwald donors)
  - Boronic acids and pinacol boronates (Suzuki acceptors)
  - Pyridines, pyrimidines with substituents
  - Pyrazoles, imidazoles, oxazoles, thiazoles (covers 37% of unsolved USPTO-50k)
  - Furans, thiophenes, bromofurans, bromothiophenes
  - Pharmaceutical amines: piperidine, morpholine, piperazine (N-Boc protected)
  - Aldehydes (for reductive amination / condensation)
  - Nitriles, iodoarenes, fluorinated aromatics
- **MkDocs Material documentation site** (`docs/`)
  - Getting Started: installation, quick start
  - API Reference: Rust, Python, WASM/JavaScript
  - Examples: Aspirin, drug-like molecules
  - Benchmark page with USPTO-50k results
- **WASM interactive playground** (`docs/playground/index.html`)
  - Accessible at `https://kent-tokyo.github.io/renkin/playground/`
  - Preset examples: pharmaceuticals, Suzuki products, aryl C-N/C-O, Wittig
- **GitHub Actions `docs.yml`** — automatic WASM build + MkDocs → GitHub Pages on every push to master
- **Diagnostic trace tests** (`src/trace_test.rs`) — pipeline debug utilities for issue investigation

### Changed
- **USPTO-50k benchmark: 2.6% → 5.0%** (500-molecule sample, depth=2, beam=20)
  - +12 newly solved molecules from new rules and expanded building block stock
- chematic dependency updated to 0.4.10 (minor upstream fixes)
- `docs/playground/index.html` uses DOM-safe textContent everywhere (no innerHTML with user data)

### Fixed
- `cargo fmt` formatting in `src/bin/benchmark.rs` (caused CI failures)

---

## [0.1.0] — 2026-06-20

Initial public release. Published to [crates.io](https://crates.io/crates/renkin), [PyPI](https://pypi.org/project/renkin/), and [npm](https://www.npmjs.com/package/renkin).

### Added
- **Core retrosynthesis engine** (`src/chem_env.rs`)
  - 14 SMIRKS retro-rules: ester, amide, Friedel-Crafts acylation, aryl C-N/C-O, Buchwald-Hartwig, aryl ether, Suzuki (graph-based), C-C, Wittig, reductive amination, C-N/C-O aliphatic, alcohol oxidation
  - Fragment sanitization: `.`-split canonical SMILES + `standardize(remove_explicit_h)` + open-chain aromatic filter
  - Building block identity via VF2 substructure matching (`parse_smarts` + `find_matches`) — immune to canonical SMILES ordering issues
  - `HashMap<(atom_count, bond_count), Vec<BbEntry>>` pre-filter for O(1) lookup before VF2
- **Graph-based Ar-Ar cleavage** (`biaryl_cleavage`) — bridge-bond DFS correctly handles symmetric biaryls without SMIRKS BFS leakage artifacts
- **A\* / AND-OR tree search** (`src/search.rs`)
  - Priority queue (`BinaryHeap`) with closed-list deduplication
  - Degenerate-route filter (skips precursor sets containing the target itself)
  - Depth-0 routes (target is a building block)
  - Beam-width pruning (`--beam-width N`, 0 = unlimited A\*)
- **SA Score heuristic** (`src/score.rs`) — `h = Σ(1.0 + 0.5 × (sa − 1) / 9)`, admissible upper bound for A\*
- **Parallel rule application** — `rayon::par_iter()` on non-WASM; sequential fallback on `wasm32`
- **Python bindings** (`src/python.rs`) via PyO3 + maturin
  - `renkin.find_routes(smiles, depth, max_routes, beam_width, building_blocks=None) -> dict`
  - `renkin.version() -> str`
- **WASM bindings** (`src/wasm.rs`) via wasm-bindgen
  - `find_routes(target, depth, max_routes, beam_width) -> String` (JSON)
  - `version() -> String`
  - 493 KB bundle via `wasm-pack build --target web --no-default-features`
- **CLI binary** (`src/main.rs`) — `renkin --target SMILES --depth N --beam-width N`
- **Benchmark binary** (`src/bin/benchmark.rs`) — `renkin-bench --input file.smi` → JSON report
- **Browser WASM demo** (`demo/index.html`) — SmilesDrawer 2D rendering, preset examples, beam/depth controls
- **277 building blocks** (`data/building_blocks.smi`) — aliphatics, acyl chlorides, carbonyls, aryl halides, boronic acids, heterocycles, amino acids, sulfonyl chlorides, isocyanates, protecting-group reagents
- **42-molecule benchmark set** (`data/benchmark_targets.smi`) — ester/amide/C-N/C-O/Suzuki/Buchwald/Wittig coverage
- **23 unit tests** across `chem_env`, `search`, `score`
- **GitHub Actions** CI (`ci.yml`) — `cargo test` + `cargo fmt --check` on push/PR
- **GitHub Actions** Release (`release.yml`) — multi-platform Python wheels (Linux/macOS/Windows), npm WASM, crates.io on `v*` tag push
- **GitHub Secrets** configured: `PYPI_TOKEN`, `NPM_TOKEN`, `CARGO_REGISTRY_TOKEN`

### Known Limitations (v0.1.0)
- chematic issues #13 (BFS leakage) and #14 (non-deterministic canonical SMILES) are unresolved upstream — workarounds in place
- USPTO-50k success rate: 2.6% (depth=2, beam=20, 500-mol sample) — reflects 277-BB stock, not rule quality
- macOS arm64 wheel only at initial PyPI release (multi-platform added via CI for subsequent releases)

---

[Unreleased]: https://github.com/kent-tokyo/renkin/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/kent-tokyo/renkin/releases/tag/v0.1.0
