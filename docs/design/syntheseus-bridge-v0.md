# RENKIN Syntheseus Bridge — Design Doc

Status: **Phase 0-3 complete (feasibility+schema, Python exporter, Rust
normalizer + `--format syntheseus` CLI flag, 3-way conformance suite +
Playground + docs). Phase 4 (release: version bump, publish) not
started, its own future round requiring separate approval.** Real
fixtures generated for Phase 0 live at `tests/fixtures/syntheseus/0.7.2/`
(see that directory's own `PROVENANCE.md` for exact construction/
reproduction detail); §7 records how Phase 1-2's open questions were
resolved. Phase 3's own demo walkthrough:
[Audit a Syntheseus Route](../guides/syntheseus-audit-demo.md).

## 0. What this is, in one paragraph

The prior Syntheseus investigation (recorded in
`docs/guides/open-source-retrosynthesis-comparison.md` /
`docs/comparison/askcos-feasibility-issue-66.md`-adjacent history) found
"Syntheseus has no native JSON export" and treated that as a blocker.
This round's design decision, given by the user, resolves it
differently: **RENKIN owns the converter.** Syntheseus's real,
public Python interface (`Molecule`, `Bag`, `SingleProductReaction`,
`SynthesisGraph`) is sufficient to build a stable, deterministic
`syntheseus-route-v1` JSON export — confirmed this round by actually
doing it, against real (not model-searched, but genuinely real, validated)
Syntheseus objects, with zero model inference, checkpoint download, GPU,
or large-scale search anywhere in the path. This document is the
schema + architecture for that converter and the RENKIN-side adapter
that will eventually consume it; §7 lists what's still an open
decision before Phase 1 can start.

## 1. Existing-code grounding (read before designing, not after)

- **`src/bridge/aizynthfinder.rs`** is the direct precedent for
  everything Phase 2-3 will do: a `normalize_<tool>_route` function that
  turns a tool-specific parsed shape into a [`RouteDocument`]
  (`src/bridge/route_graph.rs`), fail-loud on malformed shapes (never a
  silent best-effort parse), tolerant of unknown/future JSON fields (no
  `deny_unknown_fields`), and backed by real captured fixtures with a
  `PROVENANCE.md` (`tests/fixtures/aizynthfinder/v4.4.1/`) — this round's
  own fixture directory and PROVENANCE.md follow that exact convention,
  adapted for a different (object-construction, not live-tool-capture)
  provenance story (§4 explains why).
- **`bridge::route_graph::normalize_renkin_route`**'s `build()` function
  is the actual tree-flattening algorithm Phase 2 will need to port: it
  walks a flat `steps: Vec<{target, precursors, template_id}>` list (via
  a `steps_by_target: HashMap` lookup) into a `RouteNode` tree,
  recursively, with `on_stack: HashSet<String>` cycle detection. This
  matters directly for §7's open question below: this function already
  has a real, working answer for "what happens when the same molecule is
  reachable via two different paths through the step list" — it just
  re-expands that molecule's own sub-tree independently under each
  parent, no special-casing, no error. **`syntheseus-route-v1`'s own
  `steps` field is deliberately shaped as a flat list for exactly this
  reason** — not because Syntheseus's own data model demanded it (it
  doesn't especially, see §3), but so `normalize_syntheseus_route`
  (Phase 2) can reuse this exact, already-tested algorithm with minimal
  adaptation, rather than porting a second, differently-shaped
  tree-builder.
- **`bridge::audit::AuditFindingCode`** is a closed set (`RawOutputNotDecodable`,
  `MultipleOrZeroRoots`, `CycleDetected`, `AmbiguousLeafStatus`, ...) --
  Phase 2's `normalize_syntheseus_route` should map its own malformed-input
  cases onto this **same existing enum**, not invent Syntheseus-specific
  codes, unless a real gap turns up that neither adapter's existing codes
  cover (none found in this round's fixtures — the two structural
  edge cases actually exercised, see §4, both map cleanly onto codes that
  already exist: `AmbiguousLeafStatus`, `CycleDetected`).
- **`docs/guides/audit-reproducibility-contract.md`**'s compatibility
  rule 5 ("source-tool stock claims and RENKIN's own stock verification
  are separate signals, never merged") is why `syntheseus-route-v1`'s
  `molecule_metadata[...].is_purchasable` is read the same way
  AiZynthFinder's `in_stock` already is: structural input for leaf
  detection only, never conflated with RENKIN's own `--stock` file
  verification.

## 2. Phase 0 findings (the actual feasibility investigation)

Installed `syntheseus==0.7.2` from PyPI into a clean venv and inspected
its real source, not documentation alone (`pip show syntheseus`
confirmed the install; module source read directly from
`site-packages/syntheseus/`).

- **The base package has zero model/GPU dependencies.**
  `pip show syntheseus`'s `Requires:` line is exactly
  `more_itertools, networkx, numpy, omegaconf, rdkit, tqdm` — no
  `torch`, no model-backend package at all. Model-specific code lives
  under `syntheseus.reaction_prediction.inference.*` (`chemformer`,
  `gln`, `graph2edits`, `local_retro`, `megan`, `mhnreact`, `retro_knn`,
  `root_aligned`), never imported anywhere in this spike. This alone
  answers the "can this be done without a model" question at the
  dependency-graph level, before even trying construction.
- **`syntheseus.search.graph.route.SynthesisGraph`** is the right target
  abstraction — confirmed, not assumed, by reading
  `syntheseus/search/graph/and_or.py`'s and `.../molset.py`'s own
  source: both `AndOrGraph.to_synthesis_graph()` and
  `MolSetGraph.to_synthesis_graph()` (real, public, non-underscore
  methods) convert a completed search graph — regardless of which
  algorithm produced it — down to exactly this same `SynthesisGraph`
  type. It's Syntheseus's own canonical "one clean route" shape, not a
  workaround this design invented.
- **`SynthesisGraph` is constructible with zero search/model
  involvement**, using nothing but `Molecule`, `Bag`,
  `SingleProductReaction` (all plain frozen dataclasses in
  `syntheseus.interface.*`) plus direct graph construction
  (`SynthesisGraph(root_node=...)`, then
  `graph._graph.add_edge(parent, child)` for further steps). This is
  not a private-API workaround: it is the **identical pattern**
  `AndOrGraph.to_synthesis_graph()`'s own real implementation uses
  internally (`new_graph._graph.add_node(...)` /
  `.add_edge(...)`), and it is also exactly the pattern Syntheseus's
  **own test suite** uses to build route fixtures for its own tests
  (`syntheseus/tests/search/conftest.py`'s `minimal_synthesis_graph`
  fixture) — so this spike's construction method matches both the
  library's internal production code path and its own testing
  convention, not a third, invented approach.
- **Syntheseus's own built-in structural validator caught nothing wrong**:
  `SynthesisGraph.assert_validity()` (parent/child product-membership
  checks, uniqueness checks) passed on both fixtures constructed this
  round. `is_tree()` and `get_starting_molecules()` (Syntheseus's own
  leaf-detection, used directly rather than reinvented) both behaved
  exactly as expected, including correctly reporting `is_tree() == False`
  for the deliberately-convergent fixture.
- **`syntheseus.__version__` does not exist** — a real, concrete
  discovery, not a hypothetical: the correct way to record
  `source_version` in the exporter is `importlib.metadata.version("syntheseus")`,
  confirmed to match `pip show`'s own reported version.
- **Determinism confirmed directly**: running the exporter twice against
  the identical in-memory object produced byte-identical JSON both
  times (`Bag`'s own internal `tuple(sorted(...))` storage, combined
  with `Molecule` being `order=True`, already gives deterministic
  iteration order "for free" — the exporter's own explicit sort-by-SMILES
  traversal on top of that is belt-and-braces, not strictly required by
  `Bag` alone, but keeps the guarantee explicit rather than relying on
  an implementation detail of `Bag`'s sort behavior).
- **At spike time, `0.7.2` is not the latest release** — PyPI's current
  latest is `0.8.0` (`pip index versions syntheseus`). Targeting `0.7.2`
  per the user's explicit instruction, not because it's current-latest;
  worth knowing this gap exists before Phase 1 locks in a version.

## 3. `syntheseus-route-v1` schema

```json
{
  "schema_version": 1,
  "source_tool": "syntheseus",
  "source_version": "0.7.2",
  "target": "CCOC(=O)c1ccccc1",
  "steps": [
    {
      "product": "CCOC(=O)c1ccccc1",
      "reactants": ["CCO", "O=C(O)c1ccccc1"],
      "reaction_metadata": {
        "reaction_smiles": "CCO.O=C(O)c1ccccc1>>CCOC(=O)c1ccccc1",
        "identifier": "step1",
        "template": "esterification_retro",
        "source": "...",
        "reaction_id": "..."
      }
    }
  ],
  "starting_molecules": ["CCO", "O=C(O)c1ccccc1"],
  "molecule_metadata": {
    "CCO": { "is_purchasable": true },
    "O=C(O)c1ccccc1": { "is_purchasable": true, "cost": 12.5, "supplier": "TestSupplierCo" }
  },
  "source_metadata": {
    "exporter_schema": "syntheseus-route-v1",
    "note": "..."
  }
}
```

Design choices, each grounded in §2's findings, not guessed:

- **Flat `steps` list**, mirroring RENKIN's own native `--format json`
  route shape exactly (`{target, precursors, template_id}` per step) —
  see §1's grounding note on why this lets Phase 2 reuse
  `normalize_renkin_route`'s existing tree-builder algorithm almost
  directly, rather than porting AiZynthFinder-style nested-tree logic a
  second time for a data source whose own native shape isn't nested to
  begin with.
- **`starting_molecules` recorded explicitly**, not left for the Rust
  side to infer alone — it's Syntheseus's own `get_starting_molecules()`
  output, included so Phase 3's conformance suite can cross-check RENKIN's
  independently-computed leaf set against Syntheseus's own notion of
  "starting molecule" as a real conformance signal (do they agree on
  every fixture?), not just trust one side.
- **`molecule_metadata` keyed by canonical SMILES, leaves only** — a
  molecule that's a reaction *product* has no meaningful purchasability
  question to answer; only entries for SMILES appearing in
  `starting_molecules` are emitted. `is_purchasable: null` (present, not
  omitted) is the genuinely-ambiguous case — confirmed real and
  reachable, not hypothetical (Fixture B's `"CC"` leaf).
- **`reaction_metadata.identifier` and `.reaction_id` kept as two
  distinct optional fields**, not conflated — they're genuinely two
  different fields in Syntheseus's own `Reaction` dataclass
  (`Reaction.identifier: Optional[str]` vs.
  `Reaction.metadata["reaction_id"]: int` per `ReactionMetaData`'s own
  `TypedDict`), so the interchange schema preserves that distinction
  rather than picking one and discarding data.
- **`reaction_smiles` always present**, since it's a computed property
  on every real `Reaction` object (`rxn.reaction_smiles`), never
  missing — safe to treat as required, not optional, in the schema.

## 4. Fixtures generated this round

Two real, exporter-produced JSON files, both from genuine (validated,
not hand-typed) `SynthesisGraph` objects — see
`tests/fixtures/syntheseus/0.7.2/PROVENANCE.md` for full construction
code, checksums, and Syntheseus's own reported structural properties
for each:

- **`linear_two_leaf_route.json`** — single-step, both leaves carrying
  full `is_purchasable`/`cost`/`supplier` metadata. The "everything
  present" case.
- **`convergent_route.json`** — 2-step, deliberately non-tree (a
  molecule produced by one reaction is consumed by two different
  downstream reactions), one leaf with **no** purchasability metadata
  at all. Exercises both the genuinely-ambiguous-leaf case and the real
  structural question §7 raises.

Neither is a live Syntheseus *search* capture (that needs a model,
explicitly out of scope this round) — both are real Syntheseus objects,
validated by Syntheseus's own `assert_validity()`, exported by a real
(if spike-quality) exporter script. This distinction is documented
explicitly in the fixtures' own `PROVENANCE.md`, not glossed over.

## 5. What Phase 1-4 look like (from the user's own instruction, recorded for continuity)

- **Phase 1 — Python exporter.** A real, packaged module in RENKIN's
  Python surface (not this round's spike script). Must not depend on
  private/internal Syntheseus attributes beyond what §2 already
  confirmed stable (`Molecule`/`Bag`/`SingleProductReaction`/
  `SynthesisGraph`, and `._graph.add_edge` — the same call Syntheseus's
  own `to_synthesis_graph()` uses, but still a leading-underscore
  attribute, worth a real compatibility note in the Phase 1 doc since
  underscore-prefixed attributes carry no stability guarantee even
  when a library's own code relies on them internally). Fail-loud on
  unsupported object shapes. Deterministic node/JSON ordering (already
  proven in §2). Byte-stable output for the same input object.
- **Phase 2 — Rust adapter.** `normalize_syntheseus_route`, mirroring
  `bridge::aizynthfinder`'s shape exactly (see §1). New CLI
  `--format syntheseus`, folded into the existing strict
  `detect_audit_route_format` auto-detection (never guesses between
  ambiguous shapes). Malformed hierarchy/cycles/multiple-roots map onto
  existing `AuditFindingCode` variants (§1). Missing reaction evidence
  → `not_evaluable`, same as the AiZynthFinder adapter's own convention.
  Stock claims (`molecule_metadata[...].is_purchasable`) handled exactly
  like AiZynthFinder's `in_stock` (§1's grounding note).
- **Phase 3 — conformance + Playground.** A 3-way parity fixture set
  (RENKIN-native / AiZynthFinder / Syntheseus) analogous to
  `tests/cross_tool_audit.rs`'s existing 2-way fixtures, checked across
  root/leaf-multiset/step-count/structural-findings/element-accounting/
  policy-verdict — and, since v0.29.0 already shipped Audit Policy
  Profiles, across all 3 policies too, confirming finding-set invariance
  holds for this third adapter the same way it was proven for the first
  two. Playground gains Syntheseus as a third `--format` option.
  Compatibility matrix doc updated.
- **Phase 4 — release.** Version bump, docs, publish — same pattern as
  every prior release this project has done, its own explicit
  authorization.

## 6. Constraints reaffirmed (this round and beyond, per explicit instruction)

No checkpoint download, no model inference, no GPU, no large-scale
search, ever, for this adapter's own construction/testing. No ASKCOS
work. No Evidence Package work. No ONNX/coverage-mode/Issue #128 work.
No hand-typed-only fixtures accepted as "verified" — every fixture must
trace to a real object via a real, documented exporter run, matching
the same bar `tests/fixtures/aizynthfinder/v4.4.1/`'s real captures
already set.

## 7. Open questions for Phase 1-2 (resolution record)

1. **Convergent/non-tree routes: how should `normalize_syntheseus_route`
   handle them?** **Resolved in Phase 2, as recommended below**: `build()`
   (`bridge::route_graph`) re-expands the shared molecule's sub-tree
   independently under each parent, no special-casing — confirmed against
   the real `convergent_route.json` fixture
   (`bridge::syntheseus::tests::real_convergent_route_normalizes_by_duplicating_the_shared_subtree`,
   `tests/audit_route_cli.rs::syntheseus_convergent_fixtures_ambiguous_leaf_fails_with_two_findings`).
   Original reasoning kept below for context.

   Fixture B (§4) proves this is a real, reachable case,
   not hypothetical. §1's grounding note shows
   `normalize_renkin_route`'s existing `build()` already has a de facto
   answer — re-expand the shared molecule's sub-tree independently under
   each parent, no error, no special representation — simply by not
   doing anything special. `RouteNode`'s own tree shape has no way to
   represent a DAG node with two parents, so duplication-on-flatten is the
   only representable outcome without a bigger `RouteNode` schema change.
   This does change what a Syntheseus-sourced `AuditReport` can look like
   (the same underlying reaction step can appear twice in `steps[]`) in a
   way neither existing adapter's own fixtures have ever exercised on
   their own inputs.
2. **`._graph.add_edge` is a private-looking attribute.** **Resolved in
   Phase 1**: the production exporter (`renkin.syntheseus_exporter`) never
   touches `._graph` at all — it only reads an already-built
   `SynthesisGraph` via public methods (`root_node`, `successors()`,
   `get_starting_molecules()`, `assert_validity()`). Only the exporter's
   own *test* fixtures need `._graph.add_edge`, to construct multi-step
   graphs for testing (Syntheseus exposes no public multi-step
   constructor) — a test-only concern, not a production compatibility
   risk. No dual-version (`0.7.2`/`0.8.0`) test matrix was added this
   round; revisit if a real compatibility report comes in.
3. **Version target: `0.7.2` (named) vs. `0.8.0` (actual latest).**
   **Resolved**: `0.7.2`, matching the Phase 0 fixtures and the user's
   named target. `source_version` in every export still records the real
   installed version via `importlib.metadata.version`, never a hardcoded
   string, so a user on `0.8.0` gets an honest, self-reported value
   either way.
