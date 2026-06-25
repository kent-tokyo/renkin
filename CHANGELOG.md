# Changelog

All notable changes to RENKIN are documented in this file.  
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).  
RENKIN adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [0.1.5] — 2026-06-25

### Added
- **`--format tree`** — ASCII tree output for retrosynthesis routes:
  ```
  Route 1  [score=1.10, depth=1]
  OC(=O)c1ccccc1OC(=O)C
  └── [ester_cleavage]
      ├── OC(=O)C  ✓ BB
      └── c1cccc(c1O)C(O)=O  ✓ BB
  ```
- **`--format mermaid`** — Mermaid flowchart output (paste into GitHub/Notion for rendered diagrams)
- **`score` field in JSON output** — each route now includes `score: f64` (cumulative A* step cost; lower = better); routes are already sorted best-first
- `src/display.rs` — new module with `format_route_tree()` and `format_route_mermaid()`
- **Constraint-based search** — two new CLI flags (also available in Python API):
  - `--avoid-elements / -e "Br,I"` — drop any route whose leaf BBs contain a forbidden element
  - `--require-elements / -r "B"` — keep only routes whose leaf BB union supplies each required element
  - `chem_env::elem_symbols_to_mask()` helper maps symbol CSV → u64 bitmask (same format as `RetroRule::required_elements`)
  - `SearchConfig` gains `forbidden_elements: u64` and `required_element_present: u64` (both default 0 = no constraint)
  - Constraints compose freely: `--require-elements B --avoid-elements Br,I` narrows biphenyl from 5 routes to 1
- **`--verbose / -v`** — print search statistics to stderr after each run:
  ```
  [renkin] search complete
    nodes popped   : 7
    nodes expanded : 6
    routes found   : 5
    elapsed        : 0.04 s
  ```
  `SearchConfig.verbose: bool` (default false); does not affect stdout (JSON/tree/mermaid unaffected)
- `scripts/train_template_scorer.py` — MLP template scorer training script added to repo
- README: Constraint-based Search section with before/after example (5 routes → 1 route)

### Fixed
- `src/display.rs`: removed dead `child_prefix` variable (same expression as `rule_prefix`; suppressed with `let _ =`)
- `scripts/train_template_scorer.py`: added `result.returncode` check in `ecfp4_batch()` — subprocess failure previously silently corrupted training fingerprints
- `data/*.onnx` and `data/*.onnx.data` added to `.gitignore` (large binary weights)

---

## [0.1.4] — 2026-06-23

### Changed
- chematic updated **0.4.15 → 0.4.16**
  - Patch release; E/Z stereo filter (issue #21) remains active as of 0.4.15

### Added
- **`diaryl_sulfone_retro` rule** (graph-based) — cleaves Ar-SO₂-Ar bridge bonds into Ar-SO₂-Cl + Ar'-H;
  `build_sub_molecule_with_cl` helper added alongside existing `_with_br`
- **Building block set expanded 480 → 509** (+29 entries):
  - Ar-OCF₃ series (10 entries): `FC(F)(F)Oc1ccccc1`, 4-Br/Cl/F/NH₂/F-OCF₃ arenes, OCF₃ pyridines
  - ArCF₃ amines / halides (8 entries): ortho/meta isomers, 3-/4-aminobenzotrifluoride, etc.
  - CF₃CH₂ series (3 entries): 2,2,2-trifluoroethanol, -amine, -bromide
  - Sulfonyl chlorides (11 entries): EtSO₂Cl, PrSO₂Cl, PhSO₂Cl, TsCl, 4-Cl/F/3-F/4-OMe-PhSO₂Cl, iPrSO₂Cl, 5-Me-2-PySO₂Cl
- **E/Z stereo coverage expanded** — 3 new regression tests in `chematic_regression`:
  - `ez_stereo_e_selective_smirks`: E-SMIRKS matches E-alkene and rejects Z-alkene
  - `ez_stereo_unspecified_smirks_matches_both_geometries`: stereo-unspecified SMIRKS is permissive
  - `ez_stereo_stilbene_wittig_discrimination`: (E)/(Z)-stilbene discrimination on real molecule
- **3 regression tests for `diaryl_sulfone_retro`**: diphenyl sulfone, asymmetric sulfone, thioether guard
- **USPTO-50k benchmark**: **78.1%** (3,831/4,907) — +5 molecules vs v0.1.3 (78.0%)

---

## [0.1.3] — 2026-06-22

### Changed
- chematic dependency updated to **0.4.15** / chematic-rxn **0.4.15**
  - Issue #21 (E/Z double-bond stereo filtering in `run_reactants`) now active:
    SMIRKS templates with `/`/`\` on both sides of a double bond correctly
    filter reactants whose geometry does not match (filter/point 1).
    Transfer (point 2) and create (point 3) remain as chematic follow-up.
- Phase A full-run benchmark **top-5000 templates**: **78.1%** (3,830/4,907 — all 50 chunks ✅)
  - top-500 → top-5000: +6.0 pp improvement
  - All ~4,900 chematic-compatible templates from 5,000 extracted candidates applied
- Phase A full-run benchmark (beam=100, depth=5, top-500, Phase A): **72.1%** (3,540/4,907 — all 50 chunks complete ✅)
  

### Added
- **Phase 15 — tetrahedral `@`/`@@` stereo fully integrated** (chematic #20, fixed in v0.4.13):
  - 15.1 `stereo_templates_load_from_file_and_filter`: @/@@ templates from top-500 file load
    and correctly reject the wrong enantiomer via `apply_retro`
  - 15.2 `non_stereo_smirks_matches_both_enantiomers`: stereo-unspecified SMIRKS is permissive
  - 15.3 `stereo_transferred_to_product`: L-alanine retro confirms product retains @@ (point 2)
  - 15.3 `both_stereo_templates_are_enantiomer_selective`: R- and S-templates cross-validated
- `parse_smarts_accepts_atom_maps` extended with `[C@:1]`, `[C@@H:2]` cases
- Regression test `ez_stereo_filter_rejects_wrong_geometry` — verifies that
  a Z-selective SMIRKS `[C:1]/[C:2]=[C:3]\\[C:4]` rejects (E)-3-hexene
  reactants (chematic issue #21 regression)

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
- E/Z double-bond stereochemistry (`/`/`\`) in SMIRKS: filter active via chematic-rxn 0.4.15
  (issue #21); transfer and create (points 2/3) remain as chematic follow-up
- All benchmark numbers (47.2%, 72.1%) measured on USPTO-50k standard train/test split (same corpus).
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
