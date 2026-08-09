# 5-target smoke gate — Issue #66 comparison harness

Run before the 100-target feasibility measurement, per the frozen protocol.
Targets are drawn from `sample_full_sorted.jsonl`'s first 100 rows
(`sample_100`) wherever a category exists there; none required hand-picking
outside the sample this round.

## Category selection (from `sample_100`)

| Category | `sample_rank` | `target_id` | Canonical SMILES |
|---|---|---|---|
| Has a route | 2 | `uspto50k_test#L370` | `CC(C)CCC(C)C=O` |
| No route | 0 | `uspto50k_test#L3855` | `CC(=O)Nc1nc(C#N)c(Sc2ccccc2N)s1` |
| Stereochemistry-bearing | 8 | `uspto50k_test#L1446` | `CC/C=C\C/C=C\C/C=C\C/C=C\C/C=C\C/C=C\CCC(=O)NCCOCCNC(=O)C(C)(C)Oc1ccc(C(=O)c2ccc(Cl)cc2)cc1` |
| Multi-ring (≥3 rings) | 1 | `uspto50k_test#L2441` | `CCOc1ccc2c(c1F)OC1CC(=O)CCC21` |
| Large molecule (top decile by heavy-atom count in `sample_100`, 58 heavy atoms) | 77 | `uspto50k_test#L3262` | `O=C(OCc1ccc([N+](=O)[O-])cc1)N1CCN(C[C@@H]2C[C@H](SC(c3ccccc3)(c3ccccc3)c3ccccc3)CN2C(=O)OCc2ccc([N+](=O)[O-])cc2)CC1` |

Selection was deterministic, not cherry-picked: for "has a route" / "no
route", the first `sample_100` target (by `sample_rank`) with RENKIN's
`route_found` true/false respectively; for stereochemistry, the first
target whose canonical SMILES contains `@`/`/`/`\`; for multi-ring, the
first target with RDKit `NumRings() >= 3`; for large molecule, the single
largest by RDKit heavy-atom count within `sample_100`.

## RENKIN — pass/fail conditions

All 6 conditions verified directly against the real, built `renkin` release
binary (not synthetic fixtures) — see
`scripts/tests/test_compare_renkin_adapter.py` for the automated regression
form of this gate.

| Condition | Result |
|---|---|
| Adapter parses the route tree correctly (has-route target) | **PASS** — `route_tree_parseable=true`, `reaction_steps_parseable=true`, `all_leaves_in_configured_stock=true`, `target_element_accounting_status=accounted` |
| No-route target handled without a route tree | **PASS** — `route_found=false`, `route_tree_parseable=null` (no tree to parse), `tool_specific.renkin` diagnostics populated (`nodes_expanded`, `max_depth_reached`, etc.) |
| Timeout is enforced | **PASS** — artificially tiny deadline (0.0005s, 1.0s grace) → `run_status=timeout`; observed wall-clock (~2.26s) matches the SIGTERM→grace→SIGKILL path, not the tool's own (nonexistent) internal budget |
| Crash converts to a structured status | **PASS** — invalid `--building-blocks` path → `run_status=crashed`, stderr captured verbatim in `adapter_warnings` (`renkin_nonzero_exit`), no uncaught exception propagated |
| Stock-leaf validation works | **PASS** — verified both the full-stock case (`all_leaves_in_configured_stock=true`) and a deliberately incomplete `shared_stock`-style stock (`["CCO"]` only) correctly reports `all_leaves_in_configured_stock=false` |
| Repeating the same input twice gives consistent status + normalized route hash | **PASS** — RENKIN is fully deterministic (no RNG/seed); two independent runs of the has-route target produced byte-identical `run_status` and `normalized_route_sha256` |

## AiZynthFinder — pass/fail conditions

Run against `docker/aizynthfinder.Dockerfile` (image
`renkin-compare-66/aizynthfinder:4.4.1`) using the official public ZINC
stock + USPTO ONNX policy (downloaded via `download_public_data`,
SHA-256-recorded in `data/comparison/aizynthfinder_public_data/`). Default
search config confirmed via direct introspection of
`aizynthfinder.context.config.Configuration()` inside the container:
`iteration_limit=100`, `time_limit=120`, `algorithm=mcts`.

| Condition | Result |
|---|---|
| Adapter parses the route tree correctly | **PASS (after a real bug fix — see below)** — confirmed structurally against the real `aizynthcli --output .../output.json` shape; `normalize_aizynthfinder_route` correctly walks the mol/reaction-interposed tree once the adapter reads the right record |
| No-route case handled without a route tree | **PASS** — confirmed against a genuinely hard target from the sample (`is_solved=false` with a non-empty best-effort `trees` list, correctly recorded as `route_found=false` without attempting to parse a tree) |
| Timeout is enforced (`docker kill` on the container, not just the local `docker run` client) | **PASS** — artificially tiny deadline (1.5s, 3s grace) → `run_status=timeout`, observed wall-clock ≈1.56s (matches the deadline, not the 120s default search budget); `docker ps -a` confirmed no orphaned container survives (`docker rm -f` cleanup in a `finally` block runs regardless of outcome) |
| Crash converts to a structured status | **PASS** — an early adapter bug (relative host path passed to `docker run -v`, which Docker silently reinterprets as an invalid named-volume request rather than a mount) was itself caught cleanly: `run_status=crashed`, full `docker` stderr captured verbatim in `adapter_warnings`. Fixed by resolving both the public-data-dir and per-target workdir to absolute paths before constructing the mount arguments (`os.path.abspath`) — a real bug this smoke gate's crash-handling check caught before the 100-target run, not a synthetic exercise |
| Stock-leaf validation works | **PASS (after a scope fix — see below)** — verified against the RENKIN-402-compound matched stock (independently re-verified) and against native mode's tool-trusted fallback (see below) |
| Repeating the same input twice: **status stability only**, never route-hash equality (AiZynthFinder's MCTS search has no documented seed control) | Not run as a standalone repeat — the 100-target run itself is a single pass per target per the frozen protocol (n=100 round uses one run per target); repeat-run variance characterization is explicitly deferred to a future round, per the protocol doc |

Peak RSS for a real run: ~3.0-3.7 GB (`docker_stats_sampled`) — reflects the ~650 MB ZINC stock plus USPTO ONNX models loaded into the container's memory, not a leak.

### Two real bugs this smoke gate caught before the 100-target run

**Bug 1 — wrong output envelope shape (critical, silently made every row report "no route").**
`aizynthcli --output ....json` writes a pandas `to_json(orient="table")`
envelope: `{"schema": {...column definitions...}, "data": [<one record per
target>]}` — **not** a bare list of records and **not** a bare per-target
dict, as the adapter originally (wrongly) assumed. The bug was only caught
because the smoke gate's "no-route case" check used aspirin — a
famously trivial synthesis target — as the expected "definitely finds
something" reference case, and it silently reported no route. Direct
inspection of `docker run ... aizynthcli ...` output revealed the real
envelope shape and an `is_solved` boolean field. **A second, compounding
design error was found at the same time**: a non-empty `trees` list does
**not** imply `is_solved=true` — AiZynthFinder always returns its best-effort
top-N candidate routes regardless of whether any is fully stock-terminating,
so "trees non-empty" is not a valid proxy for "solved" either. Fixed by (a)
reading `parsed["data"][0]` defensively, and (b) setting `route_found =
record["is_solved"]`, never `len(trees) > 0`. Re-verified after the fix:
aspirin (`CC(=O)Oc1ccccc1C(=O)O`), acetanilide, N-methylacetanilide, methyl
benzoate, and benzamide all now correctly report `route_found=true` with
fully-parseable, target-element-accounted routes.

**Bug 2 — native mode's "configured stock" was wrongly RENKIN's 402
compounds, not AiZynthFinder's real ~17.4M-compound ZINC stock.**
`docker run aizynthcli`'s own startup log reports `Compounds in stock:
17422831` for the default ZINC configuration — several orders of magnitude
too large to canonicalize and independently re-verify per row this round.
The harness originally (wrongly) passed RENKIN's 402-compound building-block
list as the "configured stock" for AiZynthFinder's *native*-mode rows too,
which would have made `all_leaves_in_configured_stock` fail almost every
row for a reason having nothing to do with AiZynthFinder's actual answer.
Fixed: native mode now passes an empty stock list, which the adapter
interprets as "trust the tool's own per-leaf `in_stock` claim instead of an
independent check this round can't practically run at that scale" — with an
explicit `adapter_warning`
(`native_stock_trusted_not_independently_verified`) on every native-mode
row so this is never silently conflated with shared-stock mode's genuine
independent re-verification (393 compounds, small enough to canonicalize
and check directly, same as the RENKIN adapter's own stock-leaf check).

## Shared-stock construction round-trip (Arm B smoke check)

**Superseded approach, historical note:** an earlier version of this arm
converted `data/building_blocks.smi` to an AiZynthFinder HDF5 stock via
`smiles2stock`, which silently dropped directional (E/Z) bond stereo for
fumaric acid and left `roundtrip_identity_confirmed=false` — accepted at
the time via a `roundtrip_identity_confirmed_modulo_stereo_layer` exception,
which is not an acceptable basis for a "shared stock" claim. This has been
replaced entirely; see `scripts/compare_shared_stock.py` and the comparison
guide's "Provenance" section for the current construction.

`scripts/compare_shared_stock.py` parses `data/building_blocks.smi`'s 449
non-comment lines directly with RDKit and writes the resulting InChIKey
table straight to HDF5 — bypassing `smiles2stock`'s own reader entirely.
Confirmed empirically:

- AiZynthFinder's default HDF5 stock format keys molecules by **InChIKey**,
  not SMILES, and its `InMemoryInchiKeyQuery` loader reads a hand-built
  HDF5 (no `smiles2stock`-specific structure required) with zero special
  handling — validated on a toy 4-compound stock against the real container
  and a real target (acetanilide) before building the full stock.
- 393 unique compounds; 9 source lines excluded because RDKit itself cannot
  parse them (3 unambiguous syntax errors, 6 aromaticity/kekulization-
  ambiguous heterocycles — see the comparison guide's "Known gaps"); 47
  further lines collapse into an already-seen InChIKey (duplicates).
- The read-back check (write the HDF5, read it back inside the same
  container) found **zero** missing/extra keys —
  `roundtrip_identity_confirmed=true`, with no exception needed: fumaric
  acid (`OC(=O)/C=C/C(=O)O`) now correctly keeps its stereo-bearing InChIKey
  because there is no separate lossy conversion step left to flatten it.

The AiZynthFinder native-mode 100-target run is executed in the same pass
as this smoke gate, not as a separate throwaway exercise — the smoke-gate
targets above are drawn from the same `sample_100` the formal run uses.
