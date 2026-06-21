# Changelog

All notable changes to RENKIN are documented in this file.  
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).  
RENKIN adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [0.1.2] — 2026-06-22

### Added
- **Phase A: Template frequency weighting** — `RetroRule.weight = ln(count+1)` from USPTO-50k
  training set; `template_bonus` reduces beam step_cost by up to 0.2 for high-frequency templates
  - Raises USPTO-50k performance: 52% → **71%** (100-molecule confirmed, full run in progress)
  - Ablation control (bonus disabled): 52%, confirms +19 pp is real
  - Methodology matches AiZynthFinder's neural template scoring (training-set frequency → inference-time priority)
- **`RetroRule.required_elements: u64`** — bitmask of atomic numbers required for a rule to match;
  skips impossible rules before `apply_retro` (`required_elements_from_smirks` at load time,
  `elem_mask_from_smiles` at search time); no false negatives by design
- **`ChemEnv::is_building_block_smiles`** — O(1) HashSet lookup for already-canonical SMILES;
  `is_bb` in search uses this as a fast path with VF2 fallback preserved for correctness
- **top-5000 template extraction** — `data/templates_extracted_5000.smi` (5,000 templates from
  USPTO-50k training set via `scripts/extract_templates.py --top 5000`)
- **chematic issue #21 resolved** — E/Z double-bond stereochemistry (`/`/`\`) in SMIRKS:
  filter (point 1) implemented upstream; reactants with mismatched E/Z geometry are now rejected.
  Pending chematic release and RENKIN Phase 15 integration (transfer/create remain as follow-up).

### Changed
- **`split_fragments` de-duplicated canonicalization** — removed redundant second `canonical_smiles`
  call and `parse` re-parse per fragment; `std_mol` used directly as `PrecursorMol.mol`
- **`load_rules_from_file` now parses frequency count** (tab-separated second column) and sets
  `weight = ln(count + 1)` on each extracted template; hand-crafted rules default to `weight = 1.0`
- **`default_rules()` refactored** — uses `rr(name, smirks)` helper for brevity; comments preserved;
  `required_elements` computed at construction via `required_elements_from_smirks`
- chematic dependency updated to **0.4.14**
  - Issue #18 (bracket atom notation `[O]`/`[N]`) fixed
  - Issue #19 (`parse_smarts` atom-map notation `:N`) fixed → template validation now uses
    `parse_smarts` directly instead of probe-molecule run
  - Issue #20 (tetrahedral `@`/`@@` in `run_reactants`) fixed in v0.4.13

### Known Limitations
- WASM playground uses 31 hand-crafted rules only (size/bindgen constraints)
- Tetrahedral stereochemistry (`@`/`@@`) fixed in chematic v0.4.13; RENKIN Phase 15 integration pending
- E/Z double-bond stereochemistry (`/`/`\`) in SMIRKS: filter fixed upstream (chematic #21);
  pending release + RENKIN Phase 15 integration (transfer/create remain)
- All benchmark numbers (47.2%, 71%) measured on USPTO-50k standard train/test split (same corpus).
  Out-of-distribution generalization not yet evaluated.

---

## [0.1.1] — 2026-06-22

### Added
- **Auto template extraction pipeline** (`scripts/extract_templates.py`)
  - rdchiral extraction from USPTO-50k training set (40,008 reactions)
  - Constraint stripping for chematic compatibility (D/H0/+0/`;` removed)
  - top-500 → 283 chematic-compatible templates (`data/templates_extracted.smi`)
- **`--templates` flag** for CLI (`renkin`) and benchmark (`renkin-bench`)
  - `load_rules_from_file()` validates each template via `run_reactants` probe
- **Chunked benchmark runner** (`scripts/run_benchmark_chunks.sh`)
  - Resumable 100-mol-per-chunk evaluation with per-chunk JSON output
  - Fixed Python code injection vulnerability (file path via `sys.argv`, not string interpolation)
- **6 additional hand-crafted rules** (total: 31, was 21)
  - `boc_deprotection_retro`, `cbz_deprotection_retro` (graph-based)
  - `sonogashira_retro`, `sulfonamide_retro`, `n_benzylation_retro`
  - `grignard_addition_retro`, `claisen_retro`, `michael_retro`
  - `acyl_chloride_from_acid`, `heck_retro_terminal`, `cc_single_cleavage`
- **Playground presets** — Acetamide + Haber-Bosch annotation, i18n (EN/JA/ZH)
- **Regression tests** for chematic Bug #13 and Bug #14 (both fixed in 0.4.12)

### Changed
- **USPTO-50k benchmark: 7.5% → 47.2%** (full 4,907 molecules)
  - 7.5%  — 31 rules, depth=3
  - 27.8% — 222 rules (31+191 auto), depth=3
  - 38.9% — 222 rules, depth=5
  - **47.2% — 314 rules (31+283 auto), depth=5** ← current best
  - Surpasses ASKCOS (41%) and AiZynthFinder lower bound (45%)
- **`ChemEnv` BB lookup**: VF2-only → canonical-SMILES `HashSet` (O(1), scales to millions)
  - Removed double-pass normalization workaround (chematic Bug #14 fixed in 0.4.12)
- **`RetroRule`**: `&'static str` → `String` (supports runtime-loaded templates)
- chematic dependency updated to **0.4.12** (Bug #13 BFS leakage + Bug #14 canonical SMILES fixed)
- RENKIN acronym restored in README and playground title
- SMILES label font size increased (0.72 → 0.80 rem) for readability

### Fixed
- Shell code injection in `run_benchmark_chunks.sh` (file path interpolated into `python3 -c` string)
- Stale `chematic Bug #14` reference in `ChemEnv` struct docstring
- `cargo fmt` / clippy warnings throughout

### Security
- `run_benchmark_chunks.sh`: file paths now passed via `sys.argv` / `jq` argument, never interpolated into Python code strings

### Known Limitations
- WASM playground uses 31 hand-crafted rules only (auto-extracted templates not bundled — size/bindgen constraints)
- Tetrahedral stereochemistry (`@`/`@@`) in SMIRKS — fixed in chematic v0.4.13 (issue #20); RENKIN Phase 15 integration pending
- E/Z double-bond stereochemistry (`/`/`\`) in SMIRKS — filter fixed in chematic (#21); pending release + RENKIN Phase 15 integration

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

[Unreleased]: https://github.com/kent-tokyo/renkin/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/kent-tokyo/renkin/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/kent-tokyo/renkin/releases/tag/v0.1.0
