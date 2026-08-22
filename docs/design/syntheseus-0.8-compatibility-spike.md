# Syntheseus 0.8.0 Compatibility Spike — v0.31.0 Phase 1

Status: **Compatibility spike complete. This is PR1's own report — a
test/verification round only, no production code change.** Scope is
exactly what was authorized: artifact provenance, dual-version API audit,
an atom-mapping feasibility spike, real-object fixtures for `0.8.0`, and
this report. No exporter code change, no mapping model, no version bump,
no merge/tag/publish this round — those are PR2-4, each a separate future
approval.

## 0. Why this round exists

`pyproject.toml`'s `syntheseus` optional extra is pinned exactly:
`syntheseus==0.7.2`. At the time v0.30.0 shipped, PyPI's actual latest was
already `0.8.0` (recorded, not glossed over, in
`tests/fixtures/syntheseus/0.7.2/PROVENANCE.md`). A user who already has
`syntheseus==0.8.0` installed and runs `pip install renkin[syntheseus]`
would be forced into a downgrade or a dependency conflict — a real
adoption barrier for exactly the audience RENKIN Bridge is trying to
reach. This round answers, with real artifacts and real objects rather
than assumption: **is the existing, unmodified exporter actually
compatible with `0.8.0`, and does Syntheseus carry any real atom-mapping
signal this adapter could use for forward validation?**

## 1. Artifact provenance

Both versions installed from the **exact PyPI wheel artifact**, not
"whatever `pip install syntheseus==X` resolves to right now" — downloaded
once via `pip download --no-deps` and pinned by SHA-256 for the rest of
this spike.

| Version | Wheel | SHA-256 (wheel) | sdist | SHA-256 (sdist) |
|---|---|---|---|---|
| `0.7.2` | `syntheseus-0.7.2-py3-none-any.whl` | `687030dfa218c7155d164cd26a2ee31dae17f56a5caa4170651e23de6e956aeb` | `syntheseus-0.7.2.tar.gz` | `7fa92d69e1eac66b431e451ca2434f778ab8d1b9fb52eea8e9bb4eba21ecc7f6` |
| `0.8.0` | `syntheseus-0.8.0-py3-none-any.whl` | `c9bf6ea244badb209b7101a2d86b2b7ab40132b636e58bf09040dd2e7a66d32b` | `syntheseus-0.8.0.tar.gz` | `bdf97b0fe184dc594ba7f8903ef29dba15370b1d73bf9f6847b0b90c6d447f39` |

`pip index versions syntheseus` at spike time: `0.8.0, 0.7.2, 0.7.1,
0.7.0, 0.6.0, 0.5.0, 0.4.1, 0.4.0, 0.3.0` — `0.8.0` confirmed still latest,
`0.7.2` confirmed still resolvable (not yanked).

## 2. Python / dependency compatibility

Both wheels: `Requires-Python: >=3.8`, `Metadata-Version: 2.4`, identical
`Requires-Dist` base runtime deps: `more_itertools`, `networkx`, `numpy`,
`omegaconf`, `rdkit`, `tqdm` — no `torch`, no model-backend package, in
either version (confirmed by reading each wheel's own `METADATA` file
directly, not `pip show` after the fact). `0.8.0` adds one new optional
extra, `retrochimera` (a single-step model package, `extra ==
"retrochimera"`), and `pytest-forked` under the `dev` extra — neither
affects the base install this adapter depends on.

## 3. API audit: two clean, artifact-pinned venvs

Two isolated venvs, each containing only the pinned wheel above plus a
locally-built `renkin` wheel from this exact repo checkout (unmodified
`python/renkin/syntheseus_exporter.py`). Full introspection script:
`scripts/syntheseus_compat_introspect.py` (checked in for
reproducibility — see §7).

**Classes/methods used, all public, all confirmed identical in both
versions** (module path, constructor signature, dataclass field list,
public member list, MRO):

- `syntheseus.interface.molecule.Molecule` (`smiles`, `identifier`,
  `metadata`, equality/hashability all unchanged)
- `syntheseus.interface.bag.Bag` (frozen sorted-tuple multiset, unchanged)
- `syntheseus.interface.reaction.SingleProductReaction` /
  `.Reaction` (`reactants`, `products`/`product`, `identifier`,
  `metadata`, `reaction_smiles` property, unchanged)
- `syntheseus.search.graph.route.SynthesisGraph` (`root_node`,
  `root_mol`, `.successors()`, `.get_starting_molecules()`,
  `.is_tree()`, `.is_minimal()`, `.assert_validity()`, all unchanged)

**Only diff found**: `Reaction`/`SingleProductReaction` gained a new
public classmethod, `from_reaction_smiles(rxn_smiles: str) -> Reaction`,
in `0.8.0`. Purely additive (nothing removed or changed signature),
inspected directly (`inspect.getsource`) rather than assumed relevant:
it's a plain unmapped-SMILES constructor (`"A.B>>C"` string → `Molecule`
objects via a `.`/`>>` split) — not an atom-mapping feature, and this
exporter doesn't need it (it already has real objects in hand, not a
string to parse). Noted for completeness, not used.

**No private-API dependency in the production exporter**: confirmed by
re-reading `python/renkin/syntheseus_exporter.py` itself (unchanged this
round) — it touches only the public members listed above. The one
leading-underscore call in this codebase (`graph._graph.add_edge(...)`)
exists solely in *test* fixture-construction code
(`scripts/tests/test_python_syntheseus_exporter.py`,
`tests/fixtures/syntheseus/*/PROVENANCE.md`), never in the shipped
module — this was already true before this spike (v0.30.0 Phase 1's own
finding) and remains true in `0.8.0`.

**Malformed-graph rejection** (a real cycle: `A → B → A` via
`._graph.add_edge`): `graph.assert_validity()` raises a plain
`AssertionError` in both `0.7.2` and `0.8.0`, identically.

## 4. Exporter output: semantic diff

Ran the real, **unmodified** `renkin.syntheseus_exporter` (same wheel,
same code) against the identical real-object construction code in both
venvs — the exact same construction as
`tests/fixtures/syntheseus/0.7.2/PROVENANCE.md`'s Fixture A (linear) and
Fixture B (convergent).

**Result: byte-for-byte identical output in every field except
`source_version`** (`"0.7.2"` vs. `"0.8.0"`, exactly as the field is
designed to honestly report the real installed version — see
`syntheseus_exporter.py`'s own `importlib.metadata.version("syntheseus")`
call). Determinism (two calls against the same in-memory object produce
byte-identical JSON) confirmed independently in both venvs.

## 5. Atom-mapping feasibility spike

Classification per the four cases considered (A: source object carries a
mapped reaction; B: obtainable from model/reaction metadata; C: only an
unmapped reaction exists; D: no reaction representation at all):

**Result: Category C, in both `0.7.2` and `0.8.0`, identically — confirmed
by reading real source, not inferred from field names.**

- `Reaction.reaction_smiles` is a *computed* property
  (`syntheseus/interface/reaction.py`):
  `reaction_string(reactants_str=self.reactants_str,
  products_str=self.products_str)`, where `reactants_str`/`products_str`
  come from `molecule_bag_to_smiles()` joining each `Molecule.smiles` —
  and `Molecule.smiles` is *always* a plain RDKit canonical SMILES
  (`Molecule.__post_init__` canonicalizes on construction, with no
  provision to carry atom-map numbers). There is no code path by which
  `reaction_smiles` could contain a map number, in either version — this
  isn't "not populated in our fixtures," it's structurally impossible
  given how the property is built.
- `ReactionMetaData` (the `TypedDict` backing `Reaction.metadata`) has no
  mapped-SMILES field at all: `cost`, `template`, `source`,
  `probability`, `log_probability`, `score`, `confidence`, `reaction_id`,
  `reaction_smiles` (a plain optional *duplicate* of the computed
  property, still unmapped), `ground_truth_match`. Identical in both
  versions.
- A genuinely separate class, `syntheseus.reaction_prediction.data
  .reaction_sample.ReactionSample`, *does* have a real
  `mapped_reaction_smiles: Optional[str]` field — but this class belongs
  to the reaction-prediction **training/benchmark dataset loader**
  (used to load labeled datasets like USPTO for training/evaluating
  single-step models), not the search/route-graph layer. Confirmed by
  grep across both sdists: zero references to `reaction_sample` or
  `mapped_reaction_smiles` anywhere in
  `syntheseus/interface/reaction.py`, `syntheseus/search/graph/route.py`,
  `syntheseus/search/graph/and_or.py`, or `syntheseus/search/graph
  /molset.py` — a `SynthesisGraph`'s own `Reaction` objects have no
  path to this field, in either version.
- Several single-step model inference modules
  (`reaction_prediction/inference/{megan,graph2edits,root_aligned}.py`)
  *do* assign atom-map numbers internally (`SetAtomMapNum`) — but only as
  a private computational detail of each model's own prediction
  algorithm (e.g. root-alignment). None of it survives into the
  `Reaction`/`SingleProductReaction` objects that end up in a
  `SynthesisGraph`; nothing here is reachable from the exporter's own
  object graph either.

**Realistic path to forward-evaluable, if pursued later**: an external,
explicitly-optional enrichment step — a `MappingProvider` boundary
(unmapped reaction in, `{mapped_reaction, provider_name,
provider_version, model_or_rules_hash, confidence_or_status, warnings}`
out) — is the only path this spike found, and it requires a real
mapping tool/model as a genuinely optional dependency, with the
mapping's provenance recorded and clearly distinguished from real source
evidence. **Not attempted this round** (explicitly out of scope). No
Syntheseus route is forward-evaluable today, and this spike found no way
to make one so without adding new, optional machinery — `not_evaluable`
stays `not_evaluable`, honestly, not forced to a pass.

## 6. Cross-version conformance

Both fixtures (linear, convergent), regenerated from identical
construction code against `0.8.0`, live at
`tests/fixtures/syntheseus/0.8.0/` (own `PROVENANCE.md`, own SHA-256).
Semantically identical to their `0.7.2` counterparts in every field
except `source_version` — verified by an automated diff, not eyeballed.
Same `is_tree`/`is_minimal`/`get_starting_molecules()` results reported
by Syntheseus itself in both versions.

## 7. Test results

`scripts/tests/test_python_syntheseus_exporter.py` (unmodified test
*logic*, made version-aware: `FIXTURE_DIR` now resolves from the actually
installed `syntheseus` version rather than a hardcoded `0.7.2`) — **5/5
pass against both `env072` and `env080`** (two clean venvs, artifact-pinned
as in §1, `renkin` built from this exact unmodified checkout).
`.github/workflows/ci.yml` gained a `syntheseus-compat-matrix` job running
this exact suite against both `0.7.2` and `0.8.0` in separate jobs (not
one job reusing a mutated environment).

Introspection/comparison scripts used to produce this report are not
committed as throwaway spike code — `scripts/syntheseus_compat_introspect.py`
is checked in so this comparison is rerunnable against a future version
without re-deriving it from scratch.

## 8. Conclusion

**Verified against Syntheseus 0.7.2 and 0.8.0.** The existing,
unmodified `renkin.syntheseus_exporter` requires zero code changes to
support `0.8.0` — every class/method it touches is public and unchanged
between the two versions, and real-object output is semantically
identical. Widening `pyproject.toml`'s `syntheseus` extra pin (currently
`==0.7.2`) is now backed by real, dual-version evidence rather than an
assumption — left for PR2 per this round's explicit scope (no production
change in PR1). No path to Syntheseus forward-evaluability was found
within the base package in either version; that remains an honestly
unresolved gap, not silently declared fixed.

Phase 1 can continue: no blocker found for PR2 (support `0.8.0` in the
published pin) or PR4 (docs). PR3 (forward-evaluable Syntheseus routes)
has no green light — per this round's own success condition, it should
not be opened until a genuine mapping-provenance path is secured, which
this spike did not find within scope.
