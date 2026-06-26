# Changelog

All notable changes to RENKIN are documented in this file.  
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).  
RENKIN adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [0.15.2] — 2026-06-26

### Changed
- **README Roadmap** — moved 3 completed items from `[ ]` to `[x]`: Cargo workspace, `renkin-forward predict`, `renkin-forward validate`.
- **README / README_ja Competitive Landscape** — updated RENKIN's template entry from `rdchiral (5,000)` to `rdchiral (5k default; 50k via --templates)`.
- **README Pipeline Examples** — added 3 concrete CLI pipeline examples: route cost scoring, forward validation pipe, bond-center index.
- **Citation** — updated `version` and `url` to reference v0.15.2 release.

---

## [0.15.1] — 2026-06-26

### Fixed
- **`renkin-forward validate` stdin support** — `--route-json` is now optional; omit it to read JSON from stdin, enabling `renkin ... --format json | renkin-forward validate` pipelines.
- **`renkin-forward validate` JSON format** — now accepts both a route object `{"steps":[...]}` and the full `find_routes` output `{"routes":[{"steps":[...]}]}`; the first route is used automatically.
- **`docs/benchmark.md`** — "Latest Results (v0.2.1)" updated to "v0.15.0".

### Changed
- **README.md / README_ja.md Key Features** — updated to reflect v0.15.x capabilities: 50k templates, route cost scoring, forward validation, PaRoutes benchmark, Retro\* hooks, atom balance checker, procedure hints, MCP tools.
- **docs/index.md Key Features** — same update.

---

## [0.15.0] — 2026-06-26

### Added
- **`validate_route` MCP tool** — find the best retrosynthetic route for a SMILES and validate it: per-step atom balance check (target_MW ≤ Σ precursor_MW) + confidence/probability summary. Usable from Claude Desktop.
- **`estimate_diversity` MCP tool** — find N routes for a SMILES and report route diversity score (1 - avg pairwise Jaccard of building-block sets) plus building block breakdown per route.
- **Tool dispatch fix in `renkin-mcp`** — `tools/call` now correctly dispatches by `params.name`; previously all tool calls routed to `find_routes` regardless of tool name.
- **Template auto-detection in `renkin-mcp`** — prefers `data/templates_extracted_50000.smi` over `_5000.smi` when both are present.

### Docs
- **README.md / README_ja.md** — added benchmark scope note: USPTO-50k is a standardized sanity benchmark, not proof of broad real-world performance. Reaction space coverage is narrow (pharmaceutical C–C / C–N bias); OOD ChEMBL results (81.8%) contextualize generalization. Motivated by "A Critical Look at the USPTO Benchmark" literature thread.

---

## [0.14.0] — 2026-06-26

### Added
- **`ReactionStep.procedure_hint: Option<String>`** — one-line experimental procedure suggestion for the forward reaction. Populated for 19 hand-crafted rules; `None` (omitted from JSON) for extracted templates and fallback rules.
- **`procedure_hint_for_rule()`** in `search.rs` — maps rule names to brief procedural summaries (e.g. `"Combine aryl boronate + aryl halide + Pd(PPh₃)₄ in EtOH/H₂O, reflux at 80 °C."`).

### Architecture note
This is placeholder infrastructure for QFANG-style structured procedure generation. Once an ML backend (QFANG, ORD-trained model) is available, it can be plugged in via a `ReactionPrior`-style hook to populate `procedure_hint` with predicted action sequences instead of the static strings.

### Reference
QFANG (arXiv) — generates structured experimental procedures from reaction equations trained on 905k patent-derived action sequences. The `procedure_hint` field is the renkin-side receiver for that pipeline.

---

## [0.13.0] — 2026-06-26

### Added
- **Atom balance checker** — `renkin-bench` now verifies that each step of the best route satisfies `target_MW ≤ Σ precursor_MW` (within 1% tolerance). Violation signals a template that causes atoms to appear from nowhere — a defect highlighted by the CompleteRXN line of work.
- **`BenchResult.atom_balance_ok: bool`** — per-target flag (omitted when no routes found).
- **`BenchReport.pct_atom_balanced: f64`** — percentage of solved targets where the best route passes the atom balance check.

### Reference
CompleteRXN (arXiv) — reaction completion and balance validation; motivates per-step MW consistency checks in template-based planning.

---

## [0.12.0] — 2026-06-26

### Added
- **`scripts/train_template_scorer.py --reactions <file>`** — same API as `extract_templates.py --reactions`; train the scorer on the same local reactions file used for template extraction. Enables consistent 50k-template training pipeline.
- **`--dataset <hf_id>` / `--split <split>`** flags — explicit control over HuggingFace dataset (default unchanged: `bisectgroup/USPTO_50K` / `train`).
- **`--device <cpu|cuda|mps>`** — PyTorch device selection (default: `cpu`). Apple Silicon MPS or CUDA recommended for 50k-class training.
- **`--checkpoint-every <N>`** — save intermediate `.pt` checkpoints every N epochs. Checkpoint path: `{output_stem}_ep{N}.pt`. Useful for long training runs (~20-40 min on 480k reactions).
- **CosineAnnealingLR scheduler** — replaces constant LR; improves convergence stability for large output class counts (50k).
- **Model size logging** — prints total parameter count before training: `Training MLP: 2048->1024->512->N | X.XM params`.

### Usage (50k template pipeline end-to-end)
```bash
python3 scripts/extract_templates.py \
  --reactions /tmp/uspto_mit.smiles --top 50000 \
  --output data/templates_extracted_50000.smi

python3 scripts/train_template_scorer.py \
  --templates data/templates_extracted_50000.smi \
  --reactions /tmp/uspto_mit.smiles \
  --output data/template_scorer_50k.onnx \
  --device mps --checkpoint-every 10

renkin -t "Cc1ccc(-c2ccccc2)cc1" \
  --templates data/templates_extracted_50000.smi \
  --scorer data/template_scorer_50k.onnx --format json
```

---

## [0.11.0] — 2026-06-26

### Added
- **`scripts/extract_templates.py --reactions <file>`** — dataset-agnostic template extraction from a local reaction SMILES file (one `reactants>>products` per line). Enables use of USPTO-MIT or any proprietary reaction database without HuggingFace dependency at extraction time.
- **`--dataset <hf_id>` / `--split <split>`** flags — explicit control over the HuggingFace dataset to load (default unchanged: `bisectgroup/USPTO_50K` / `train`).

### Usage
```bash
# Export USPTO-MIT from HuggingFace, then extract 50k templates
python3 -c "
from datasets import load_dataset
ds = load_dataset('firechem/USPTO_MIT', split='train')
with open('/tmp/uspto_mit.smiles', 'w') as f:
    for row in ds: f.write(row['rxn'] + '\n')
"
python3 scripts/extract_templates.py \
  --reactions /tmp/uspto_mit.smiles \
  --top 50000 \
  --output data/templates_extracted_50000.smi
```

### Reference
- USPTO-MIT (~480k reactions) is the standard large-scale benchmark for retrosynthesis template extraction. Using it as source is expected to yield 20k–50k unique simplified templates vs. 3k–8k from USPTO-50k.

---

## [0.10.0] — 2026-06-26

### Added
- **PaRoutes benchmark adapter** — `renkin-bench --input-format paroutes` reads the PaRoutes JSON format (Genheden et al., 2022). Each entry is a mol/reaction route tree; targets and ground-truth synthesis depths are extracted automatically.
- **`--input-format smi|paroutes`** CLI flag for `renkin-bench` (default: `smi`, existing behaviour unchanged).
- **`BenchResult.gt_depth`** — ground-truth synthesis depth from PaRoutes (omitted in smi mode).
- **`BenchResult.depth_delta`** — `renkin_depth - gt_depth` per solved target (omitted in smi mode).
- **`BenchResult.route_diversity`** — route diversity score ∈ [0, 1]: `1 - avg_pairwise_Jaccard` of building-block sets across returned routes (omitted when fewer than 2 routes found).
- **`BenchReport.avg_route_diversity`** — mean diversity over targets with ≥ 2 routes.
- **`BenchReport.avg_depth_delta`** — mean depth delta over solved PaRoutes targets (0.0 in smi mode).

### Reference
- PaRoutes (Genheden et al., 2022) — multi-step retrosynthesis benchmark with 10 k ground-truth routes.
- Syntheseus (Maziarz et al., 2023) — standardised retrosynthesis evaluation framework (solved rate, route length, diversity).

---

## [0.9.0] — 2026-06-26

### Added
- **`ReactionPrior` trait** — pluggable template scoring for A\* expansion (Retro\*-style). `fn prior(&self, template_name: &str, target_smiles: &str) -> f64`. Implement to substitute frequency weighting with a neural reaction scorer.
- **`FrequencyPrior`** — default implementation using log-frequency weights (same behavior as pre-v0.9). Constructed via `FrequencyPrior::from_rules(rules)`.
- **`SearchConfig.reaction_prior: Option<Arc<dyn ReactionPrior>>`** — `None` = `FrequencyPrior` behavior (default).

### Architecture
With v0.8.0 `MoleculeValueEstimator` + v0.9.0 `ReactionPrior`, the Retro\* dual-hook architecture is complete:
- **Value hook**: how hard is this molecule to synthesize? (`MoleculeValueEstimator`)
- **Prior hook**: how likely is this template to work here? (`ReactionPrior`)

### Reference
Retro\* (ICML 2020) — neural-guided AND-OR tree search with molecule value + reaction prior.

---

## [0.8.0] — 2026-06-26

### Added
- **`MoleculeValueEstimator` trait** — pluggable A\* heuristic (Retro\*-style). Implement to substitute SA Score with a neural value function without changing the search algorithm. `SaScoreEstimator` is the default implementation (same behavior as before).
- **`SearchConfig.value_estimator: Option<Arc<dyn MoleculeValueEstimator>>`** — `None` = default SA Score behavior.
- **`ReactionStep.reaction_family: Option<String>`** — human-readable reaction family for each synthesis step (e.g. `"suzuki_coupling"`, `"esterification"`, `"buchwald_hartwig"`). `None` for extracted templates without manual assignment.

### Reference
Retro\* (ICML 2020) — pluggable value estimator architecture for AND-OR tree search.

---

## [0.7.0] — 2026-06-26

### Added
- **`Route.route_cost: f64`** — estimated synthesis cost: `Σ(BB complexity or price) + step_count × 0.5`. Lower = cheaper / simpler route.
  - Default (no price file): uses SA Score as BB complexity proxy (`chematic::chem::sa_score`).
  - With `--bb-prices path.csv`: uses actual prices from CSV (`SMILES,price_per_gram`); unmatched BBs fall back to SA Score.
- **`--bb-prices <path>` CLI flag** in `renkin` and `renkin-bench`.
- **`bb_prices_path` parameter** in `renkin.find_routes()` Python API.
- **`best_route_cost` / `avg_route_cost`** in benchmark JSON output.

### Changed
- Roadmap item "Route cost scoring" is now complete ✓.

---

## [0.6.0] — 2026-06-26

### Added
- **`renkin-forward` CLI binary** — standalone tool in `crates/renkin-forward/`:
  - `renkin-forward predict --reactants "A" "B" [--templates file.smi] [--max-results N]` — predict products from reactants
  - `renkin-forward validate --route-json '...' [--templates file.smi]` — validate a retrosynthetic route step-by-step; `verified=true` when forward prediction reproduces the target
- **`renkin.predict_forward()`** Python API — predict products inline (no circular dep; logic inlined in python.rs)
- **`renkin.validate_forward()`** Python API — validate a route JSON object returned by `find_routes()`

### Reference
ReactionT5 / Chemformer / Molecular Transformer — forward validation pattern adapted as rule-based (no ML).

---

## [0.5.0] — 2026-06-26

### Added
- **Bond-center template index** (`TemplateBondIndex`) — RetroKNN-inspired, ML-free template retrieval. Indexes templates by the element-pair bonds their SMIRKS patterns can break. At search time, only templates relevant to bonds present in the target molecule are tried, skipping irrelevant SMARTS matching.
- **`--retrieval-top-k N` flag** (CLI and benchmark) — enables bond-center retrieval, capping SMIRKS-matched candidates at N per expansion step (sorted by frequency weight). Graph-based and fallback rules are always included. Default 0 = disabled (all templates tried).
- **`bond_pairs_from_smirks()`** in `chem_env` — extracts `(min_elem, max_elem)` pair signatures from a SMIRKS reactant pattern. Reuses the existing element lookup table from `required_elements_from_smirks`.
- **`SearchConfig.retrieval_top_k`** field (default 0).

### Reference
RetroKNN (arXiv 2022) — local reaction template retrieval via atom/bond-environment stores.

---

## [0.4.0] — 2026-06-26

### Added
- **`ReactionStep.step_confidence`** — per-step template confidence (`rule_weight / max_rule_weight`). Hand-crafted rules yield equal values; extracted templates are differentiated by training frequency.
- **`Route.success_probability`** — product of step_confidence values across all steps (Retro-prob style). Estimates the probability that every step in the route succeeds. Single-step routes equal their step_confidence; multi-step routes decay multiplicatively.
- **`joint_success_probability`** in top-level JSON output — `1 − Π(1 − p_i)` over all returned routes: probability at least one route succeeds.
- **Benchmark enrichment** (`renkin-bench`): `nodes_expanded`, `best_confidence`, `best_success_prob`, `best_convergency` per target; `avg_nodes_expanded`, `avg_confidence`, `avg_convergency`, `avg_success_prob` in summary.

### Reference
Retro-prob (arXiv 2022), Syntheseus (arXiv 2023), PaRoutes (arXiv 2022) — probabilistic route scoring and Syntheseus-style benchmark metrics.

---

## [0.3.0] — 2026-06-26

### Added
- **Reaction conditions** (`conditions` field on each route step) — rule-based catalyst / solvent / temperature suggestions for all 29 hand-crafted retro rules. Extracted templates return `null` (conditions unknown without ML). No new dependencies; pure Rust lookup.
- **Atom economy** (`atom_economy: f64` on each route step) — `MW(target) / Σ MW(precursors) × 100`. Measures what fraction of precursor atoms end up in the desired product (green chemistry metric; OSS competitors do not expose this).
- **Convergency score** (`convergency: f64` on each route) — `1.0` = all branches same depth (parallel synthesis possible); `0.0` = purely linear route. Computed from leaf-depth variance in the synthesis tree.

### Changed
- `ReactionStep` gains `conditions` and `atom_economy` fields (additive; JSON consumers unaffected, Rust struct literals must add fields)
- `Route` gains `convergency` field (additive)

---

## [0.2.1] — 2026-06-26

### Fixed
- Sync `pyproject.toml` version to `0.2.1` (was stuck at `0.1.0`, causing maturin to publish `0.1.0` wheels and PyPI to skip them as already-existing — Python users never received v0.2.0)
- `docs/benchmark.md`: version header updated from v0.1.8 → v0.2.1; comparison table updated
- `docs/api/python.md`: `renkin.version()` example updated from `'0.1.0'` → `'0.2.1'`

---

## [0.2.0] — 2026-06-26

### Breaking
- `find_routes()` now returns `Result<(Vec<Route>, SearchStats)>` instead of `Result<Vec<Route>>`

### Added
- `Route.confidence: f64` — template frequency ratio (0 = rare templates, 1 = maximally common)
- `SearchStats { nodes_expanded: u64 }` — diagnostic stats returned with every search
- JSON/Python output includes `diagnostics: { nodes_expanded }` when `routes_found == 0`
- In-search pruning for `--avoid-elements`: expansions where a BB precursor contains a forbidden element are skipped before being pushed onto the heap
- 4 new regression tests: confidence range, stats non-zero on failure, pruning correctness, tuple return

### Changed
- README: constraint description updated to reflect dual-layer enforcement (in-search pruning + post-filter)

---

## [0.1.8] — 2026-06-26

### Changed
- **Benchmark comparison language softened** — replaced "exceeds AiZynthFinder/Retro\*" with explicit "not a matched-condition comparison" note; added evaluation definition (what "solved" means)
- **Version sync** — README/README\_ja citation, docs/benchmark\*, docs/index.md, docs/api/python.md all updated to v0.1.8 and 509 BBs
- **`building_blocks` in JSON** — now documented in Key Features table

### Fixed
- docs/index.md: `20 reaction rules` and `480+ building blocks` updated to reflect actual CLI capability (5,000 templates via `--templates`, 509 BBs)

---

## [0.1.7] — 2026-06-26

### Added
- **`renkin-mcp` binary** — MCP server (JSON-RPC 2.0 over stdio) for AI agent integration:
  - Tool `find_routes` with `smiles`, `depth`, `max_routes`, `avoid_elements`, `require_elements` params
  - Returns ASCII tree output + `building_blocks` list per route
  - Auto-loads `data/building_blocks.smi` / `data/templates_extracted_5000.smi` if present
  - Register in Claude Desktop: `{"mcpServers": {"renkin": {"command": "/path/to/renkin-mcp"}}}`
  - No new dependencies (serde_json already present)

---

## [0.1.6] — 2026-06-25

### Added
- **`building_blocks` field in JSON/Python output** — each `Route` now includes `building_blocks: Vec<String>`, the leaf starting-material SMILES (no manual step parsing needed)

### Fixed
- **WASM playground crash** — `std::time::Instant::now()` panics on `wasm32-unknown-unknown`; timing and node counters are now gated behind `#[cfg(not(target_arch = "wasm32"))]`

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
- **`building_blocks` field in JSON output** — each route now includes `building_blocks: Vec<String>`, the leaf precursors (starting materials to purchase) without requiring manual step parsing
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
