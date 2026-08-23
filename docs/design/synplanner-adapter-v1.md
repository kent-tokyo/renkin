# RENKIN SynPlanner Bridge — Design Doc

Status: **Phase 0 complete (feasibility + schema + real fixtures), Phase 1
PR1 (this doc + fixtures) done. No Rust/Python/WASM/CLI code written this
round — the "今回の実行範囲" scope explicitly excludes the normalizer
implementation.** Real fixtures generated this round via SynPlanner's own
installed, real export code live at
`tests/fixtures/synplanner/v1.6.0/` (see that directory's own
`PROVENANCE.md` for exact construction/reproduction detail). §7 records
open questions Phase 1 PR2 needs to resolve before implementation starts.

## 0. What this is, in one paragraph

Unlike Syntheseus (no native export at all — RENKIN had to invent
`syntheseus-route-v1`), SynPlanner **does** ship a real, native route
export function (`synplan.chem.reaction.routes.write_routes_json`). This
round confirmed its exact contract by installing the real package and
running real chemistry through its real code (not guessed from the PyPI
page, not hand-typed JSON) — see
`docs/design/synplanner-v1.6-competitive-baseline.md` §1/§4 for the
artifact-provenance summary this doc builds on. This document is the
schema mapping and adapter architecture for consuming that real export
format into RENKIN's existing, unmodified `RouteDocument` pipeline; §7
lists what's still an open decision before Phase 1 PR2 (the actual Rust
normalizer) can start.

## 1. Existing-code grounding (read before designing, not after)

- **`src/bridge/route_graph.rs`** defines the **one shared internal
  representation** every adapter builds into: `RouteDocument` / `RouteNode`
  / `ReactionEvidence` / `RouteSource`. There is no per-adapter document
  type — a SynPlanner adapter reuses this exactly, adding a new
  `RouteSource::SynPlanner` variant and a new `ReactionEvidence` variant
  (see §3). `RouteSource` is currently a 3-variant enum (`Renkin`,
  `AiZynthFinder`, `Syntheseus`) whose doc comment still says "2-variant" —
  stale even before this round; whoever implements PR2 should fix that
  comment while adding the 4th variant.
- **`src/bridge/aizynthfinder.rs`** is the direct template, not
  `src/bridge/syntheseus.rs`. There are two adapter-construction patterns
  in this codebase: a private recursive tree-walker (AiZynthFinder, for
  nested-tree input) and a shared `route_graph::build()` + `StepInfo` +
  `LeafResolver` closure (RENKIN-native and Syntheseus, for flat-step-list
  input). SynPlanner's real export is `{route_id: RouteNode}` with
  `children: list[RouteNode]` nesting — structurally identical in shape to
  AiZynthFinder's `mol`/`reaction` alternating tree. **The AiZynthFinder
  pattern is the one to port**, including its fail-loud defect vocabulary
  (`RawOutputNotDecodable`, `UnparseableSmilesInRoute`,
  `DegenerateSelfReferentialStep`, `ChildlessNonLeaf`,
  `AmbiguousLeafStatus`) and its `document = parseable.then(...)` contract
  (a route with any defect is never partially trusted).
- **`src/bridge/forward.rs`**'s existing `not_evaluable(
  MissingReactionRepresentation | MissingAtomMapping)` machinery needs
  **zero new logic**. SynPlanner's schema has no atom-mapping field
  anywhere (confirmed, see §2 below) — every SynPlanner step will land in
  `not_evaluable: missing_atom_mapping` (if some reaction-SMILES-like
  representation is available) or `MissingReactionRepresentation` (if not),
  the identical honest treatment AiZynthFinder and Syntheseus already get.
  Only a new `declared_smirks` match arm may be needed if SynPlanner's
  reaction `smiles` field needs its own resolution rule (see §7).
- **`src/bridge/audit_route.rs`**'s `detect_audit_route_format` is an
  ordered, most-specific-signal-first structural sniff. A SynPlanner
  branch needs its own distinguishing top-level marker; §3 proposes one
  grounded in this round's real fixture output, not guessed.

## 2. Phase 0 findings (the actual investigation, this round)

Full detail and construction code in
`tests/fixtures/synplanner/v1.6.0/PROVENANCE.md`; summarized here for the
adapter design itself.

- **The real input contract to SynPlanner's own exporter is not a
  `RouteNode` dict** — it's `routes_dict: dict[int, dict[int,
  chython.ReactionContainer]]` (route ID → step ID → real chemistry
  object). `_make_json_v1` walks backward from the final step, matching
  each reactant against an earlier step's product by **canonicalized,
  atom-map-stripped SMILES** (confirmed empirically: two occurrences of
  the same molecule with *different* atom-map numbers still link
  correctly — `str(mol)` after `kekule()`/`implicify_hydrogens()`/
  `thiele()` drops the map). This is irrelevant to the RENKIN-side
  adapter (which only ever sees the already-exported JSON), but explains
  why SynPlanner's own atom-map numbers in the *serialized* reaction
  `smiles` string are per-reaction-local and not guaranteed consistent
  across steps for the same molecule — worth noting if a future
  atom-mapping enrichment (§7) is attempted.
- **`in_stock` is always a definite boolean through this real code path,
  never absent/`None`** — true for every leaf with no tracked producing
  reaction, false for every reaction-produced molecule. It is **not
  independently re-verified against a stock database** by the export
  function itself; "no producing reaction in the given `routes_dict`" is
  the entire criterion. Practically: RENKIN's existing `AmbiguousLeafStatus`
  finding (used for AiZynthFinder's `in_stock: None` and Syntheseus's
  `is_purchasable: None`) may **never fire** for SynPlanner-sourced
  routes captured this way. This should be a real, tracked adapter
  behavior, not silently assumed impossible — a future SynPlanner version
  or a different call site could still produce `None` in principle.
- **`rule_id`/`rule_source`/`rule_key`/`step_id`/`tree_node_id` are opt-in
  via a `route_metadata` argument to the export call**, not automatically
  derived from the `Reaction` objects — they get `dict.update()`-merged
  directly onto the reaction node (siblings of `type`/`smiles`/`children`,
  not nested under `meta`). Whether a *real planning run's* own export
  call actually populates `route_metadata` (with real rule identifiers
  from the planning tree) was not confirmed this round — the round's
  fixtures pass synthetic-but-real metadata to prove the mechanism exists
  and round-trips correctly, not to claim real planning output always
  includes it.
- **`meta` is a separate field**, sourced from `chython.ReactionContainer.meta`
  (a real, independently-settable dict on the reaction object itself) —
  distinct from `route_metadata`.
- **Top-level file shape**: `{route_id: RouteNode}` — an **object keyed by
  route-ID string**, confirmed by running the real exporter (not assumed
  from the TypedDict's own declaration, which says nothing about the
  top-level wrapper). This directly informs §3's format-detection design.
- **Malformed-route handling**: `strict=True` raises `RouteExportError`
  carrying `RouteDiagnostic(route_id, stage, message)` tuples;
  `strict=False` silently drops the malformed route from the output dict
  while still returning the same diagnostics for inspection — confirmed
  by constructing a deliberately broken reaction (zero products) and
  running both modes.

## 3. Schema mapping

### 3.1 `RouteNode` (SynPlanner) → RENKIN `RouteNode` / `ReactionEvidence`

| SynPlanner field | On node type | RENKIN counterpart | Notes |
|---|---|---|---|
| `type` | both | (structural — determines mol vs. reaction dispatch in the walker) | Exactly mirrors AiZynthFinder's `mol`/`reaction` alternation. |
| `smiles` | mol | `RouteNode.canonical_smiles` | Canonicalize on parse, same as every other adapter. |
| `smiles` | reaction | Feeds a new `ReactionEvidence::SynPlannerReaction { smiles }` variant | This is the reaction-level SMILES/SMIRKS string (real atom maps present *within* one reaction, per-reaction-local — see §2). Passed to `forward.rs`'s existing `declared_smirks`/`has_atom_mapping` machinery unchanged. |
| `children` | both | `RouteNode.children` | Direct recursive walk, matching `azf_mol_to_route_node`'s pattern. |
| `in_stock` | mol only | `RouteNode.is_stock_leaf` | Per §2, always a definite bool through the real export path — but the adapter should still accept `Option<bool>` on parse (never assume the field is always present just because this round's evidence suggests it), treating a genuinely-absent value the same `AmbiguousLeafStatus` way AiZynthFinder does. |
| `meta` | reaction only | **No counterpart today.** Proposed: fold into a new `ReactionEvidence::SynPlannerReaction { smiles, meta: HashMap<String, serde_json::Value> }` field, kept but never interpreted (matches this codebase's "tolerant of unknown/future fields" convention). | Open question in §7: worth typing further, or keep as an opaque bag? |
| `step_id` | reaction only | **No counterpart today.** | Purely informational — SynPlanner's own step numbering, not needed for RENKIN's own tree structure (which is derived from JSON nesting, not step IDs). Candidate: drop, or keep in the same opaque `meta` bag as above. |
| `tree_node_id` | reaction only | **No counterpart today.** | Same treatment question as `step_id`. |
| `rule_id` / `rule_source` / `rule_key` | reaction only | **No counterpart in `ReactionEvidence` for *any* adapter today.** | Genuinely new capability RENKIN doesn't have anywhere yet — flagged as its own open question in §7, not unique to SynPlanner (a future rule-provenance-aware finding could use this for any adapter that has it). |
| (top-level wrapper) `{route_id: RouteNode}` | — | Each value becomes one `RouteDocument`; `route_id` itself has no RENKIN counterpart (RENKIN's own multi-route reports are keyed by array position, not a source-tool ID) — candidate: thread it through as an opaque provenance string on `AuditReport`, matching how the source format string itself is already recorded. | Open question in §7. |

### 3.2 Format auto-detection design

**Proposed signal**: a top-level JSON **object** whose values are
themselves objects with `"type": "mol"` at their root, **and** whose keys
parse as non-negative integers (route IDs). This must be checked *before*
the existing RENKIN native check (`target`+`routes` keys) and the
AiZynthFinder-batch check (`schema`+`data` keys), following the codebase's
existing "most specific signal first" convention — none of RENKIN's three
existing formats produce a bare `{"<int>": {"type": "mol", ...}}` shape,
so this is unambiguous against all three, but the exact precedence
ordering should be re-verified against real code once PR2 actually writes
`detect_audit_route_format`'s new match arm (not just asserted here).

Caveat, stated plainly rather than hidden: this signal was derived from
**two hand-constructed fixtures**, not a real planning run's actual batch
output. If a real multi-route planning run's top-level JSON shape differs
(e.g. wrapped in an outer envelope with metadata, the way AiZynthFinder's
batch mode adds a Pandas `schema`+`data` wrapper around its per-target
routes), this detection signal would need revisiting against that real
shape before Phase 1 PR2 ships. Recorded as an explicit open item in §7,
not silently assumed resolved.

### 3.3 Stock / gap survey

- **Atom mapping**: confirmed absent from the schema entirely (§2). Every
  SynPlanner step lands in `forward_validation: not_evaluable`
  (`MissingAtomMapping`, since the reaction `smiles` field itself likely
  *does* carry atom maps per-reaction as SynPlanner's own internal chython
  representation does — but whether the *exported* `smiles` string
  reliably retains them the same way `format(rxn, "m")` did in this
  round's fixtures needs re-confirming against real planning output, not
  assumed identical). No new forward-validation code needed regardless —
  `forward.rs`'s existing machinery handles both the "no mapping" and
  "mapping present but this specific SMIRKS doesn't replay" cases already.
- **Stock**: same three-valued pattern as AiZynthFinder (`Option<bool>`
  on parse), even though real-path evidence suggests it's always
  populated (§2) — RENKIN's own `--stock` file cross-check
  (`validate_stock_leaves`) stays completely independent of SynPlanner's
  own claim either way, matching the existing "tool's claim is structural
  input only, never trusted as verification" pattern.
- **Rule provenance** (`rule_id`/`rule_source`/`rule_key`): genuinely new
  information type, no existing RENKIN concept to map onto. Open question
  for Phase 1 PR2 (§7) — extend `ReactionEvidence`, or drop.

## 4. Fixtures generated this round

`tests/fixtures/synplanner/v1.6.0/`:
- `route_1_two_step.json` — 2-step nested route (bromoethane → ethanol →
  chloroethane), one stock leaf, per-step rule provenance on both
  reaction nodes. `strict=True`, zero diagnostics.
- `route_3_full_fields.json` — 1-step route exercising every optional
  `RouteNode` field simultaneously (`meta`, `step_id`, `tree_node_id`,
  `rule_id`, `rule_source`, `rule_key`), non-sequential route ID (`7`,
  not `1`) to confirm the top-level key really is caller-supplied.
  `strict=True`, zero diagnostics.
- Malformed-route behavior (both `strict=True` and `strict=False`)
  reproduced and documented in `PROVENANCE.md`, not committed as a
  separate JSON artifact — matches this codebase's existing convention of
  hand-building malformed-input test cases in Rust unit tests rather than
  shipping a "malformed fixture" file (see AiZynthFinder's
  `structurally_corrupt_route_fails_loud_not_silently`).
- Determinism verified: re-running the exact same construction code
  produced a byte-identical `route_1_two_step.json`, checked before
  committing.
- **Not attempted**: a real MCTS-searched route (would require a
  downloaded trained policy/value checkpoint, explicitly out of scope
  this round) and a genuine `in_stock: null` case (per §2's finding, this
  real code path appears structurally incapable of producing one — not
  faked to fill a checklist).

## 5. What Phase 1 PR2+ look like

Not started this round. Concretely, when authorized:
- **PR2**: `src/bridge/synplanner.rs` (new file, AiZynthFinder-pattern
  recursive walker), `RouteSource::SynPlanner` +
  `ReactionEvidence::SynPlannerReaction` variants in `route_graph.rs`
  (updating the stale "2-variant" comment while there), new
  `AuditRouteFormat` variant + detection arm + dispatch arm in
  `audit_route.rs`, format allowlist updated in **both**
  `audit_route.rs` and `main.rs` (currently duplicated, per the existing
  codebase's own pattern), plus the two CLI usage strings in `main.rs`.
  In-module unit tests against this round's real fixtures (fixture-parity
  oracle pattern, matching every other adapter). No Python/WASM logic
  changes needed — both are thin passthroughs already.
- **PR3**: cross-tool structural-parity addition to
  `tests/cross_tool_audit.rs` (a 4th "same chemistry, 4 formats" case),
  CLI-level additions to `tests/audit_route_cli.rs`, Playground fixture
  example, docs.
- **PR4**: version bump / release prep (separate authorization required,
  per this project's standing rule).
- Everything in the user's own "実施しない" list for *this* round
  (RouteDeltaGraph, MCTS, model training, benchmarking) stays out of
  scope for PR2/PR3 too — those are later phases (v0.35.0+) in the user's
  own longer-term roadmap, not part of the SynPlanner bridge itself.

## 6. Constraints reaffirmed

- SynPlanner's code was never copied — only its public, real, running
  behavior was observed (real package install, real function calls, real
  source reading of the installed package). No `chython` object crosses
  into RENKIN's Rust core; the adapter (§3) only ever consumes the
  already-exported JSON, the same boundary every other adapter respects.
- Algorithm/format attribution: SynPlanner's own README/CHANGELOG citations
  (Westerlund et al. for quality scoring, Gilmullin et al. for clustering)
  are recorded in the competitive baseline doc, not reproduced or claimed
  as RENKIN's own.
- Base RENKIN install stays untouched by this investigation — the 1.6GB
  SynPlanner venv used to generate this round's fixtures was disposable
  and has already been removed (disk headroom was a live, explicit
  constraint this round: ~14GB free before install, confirmed reclaimed
  back to ~15GB after cleanup).
- No version bump, merge, tag, publish, issue close, or upstream contact
  this round.

## 7. Open questions for Phase 1 PR2 (resolution record — update in place, never delete)

1. **Format-detection signal validity against real planning output**
   (§3.2): derived from hand-constructed fixtures only. Needs
   re-verification against a real MCTS-searched route's actual top-level
   JSON shape (which itself requires the model-download step this round
   explicitly deferred) before PR2 ships the detection code as final.
2. **Does the exported reaction `smiles` field reliably retain atom maps
   from a real planning run**, the way it did in this round's
   hand-constructed fixtures (`format(rxn, "m")`)? Not confirmed against
   real search output. If yes, SynPlanner routes might actually become
   forward-evaluable (unlike AiZynthFinder/Syntheseus) — a materially
   different outcome worth re-checking before assuming `not_evaluable` is
   the final answer for every SynPlanner step.
3. **`ReactionEvidence` extension for rule provenance**
   (`rule_id`/`rule_source`/`rule_key`): extend the shared enum (usable by
   any future adapter with similar data), or keep it SynPlanner-specific,
   or drop it entirely as out-of-scope for the audit pipeline (which
   doesn't currently have any finding that would consume rule provenance)?
   Not resolved this round — a real design decision, not an implementation
   detail.
4. **`meta`/`step_id`/`tree_node_id` treatment**: opaque pass-through bag,
   or dropped entirely? Leaning toward "keep as opaque, never interpreted"
   per this codebase's existing forward-compatibility convention, but not
   finalized.
5. **Route-ID provenance**: thread the top-level `route_id` key through as
   an opaque string on `AuditReport`, or discard it? No existing RENKIN
   concept for "the source tool's own route identifier" — worth checking
   whether AiZynthFinder/Syntheseus adapters have quietly wanted this too.
6. **`in_stock` optionality**: §2/§3.3 found the real export path always
   populates it, but the adapter's parse-time type should still be
   `Option<bool>` defensively. Confirm this doesn't create a dead code
   path that never actually exercises the `AmbiguousLeafStatus` finding
   for SynPlanner — if so, is a synthetic test case (mirroring
   AiZynthFinder's explicitly-labeled non-real Fixture C) warranted for
   PR2's own test suite?
