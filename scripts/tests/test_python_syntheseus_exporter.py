"""Tests for `renkin.syntheseus_exporter` (v0.30.0 Syntheseus Bridge, Phase 1;
dual-version compatibility spike, v0.31.0 Phase 1 PR1).

Skips entirely unless both `renkin` (maturin develop) and `syntheseus`
(`pip install renkin[syntheseus]`) are importable, mirroring
`test_python_audit_route.py`'s `requires_renkin_module` pattern.

Fixture-parity tests reconstruct the exact objects documented in
`tests/fixtures/syntheseus/<installed version>/PROVENANCE.md` and assert
the packaged exporter's output is byte-identical to the committed
fixtures for *whichever syntheseus version is actually installed* --
`FIXTURE_DIR` is resolved from `importlib.metadata.version("syntheseus")`,
not hardcoded, so the same test file verifies both `0.7.2` and `0.8.0`
(run once per version -- see `.github/workflows/ci.yml`'s
`syntheseus-compat-matrix` job) without duplicating the test bodies. An
installed version with no committed fixture directory skips only the two
fixture-parity tests (nothing to compare against yet), not the whole
suite -- the construction/determinism/rejection tests still verify the
exporter behaves correctly against real objects either way.
"""

import importlib.metadata
import json
import unittest
from pathlib import Path

try:
    import renkin  # noqa: F401
    import renkin.syntheseus_exporter as syn_exporter
    from syntheseus.interface.bag import Bag
    from syntheseus.interface.molecule import Molecule
    from syntheseus.interface.reaction import SingleProductReaction
    from syntheseus.search.graph.route import SynthesisGraph

    SYNTHESEUS_IMPORTABLE = True
except ImportError:
    SYNTHESEUS_IMPORTABLE = False

requires_syntheseus = unittest.skipUnless(
    SYNTHESEUS_IMPORTABLE,
    "requires renkin[syntheseus] installed (maturin develop --features python "
    "&& pip install syntheseus==0.7.2 or 0.8.0)",
)

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
INSTALLED_SYNTHESEUS_VERSION = (
    importlib.metadata.version("syntheseus") if SYNTHESEUS_IMPORTABLE else None
)
FIXTURE_DIR = (
    REPO_ROOT / "tests" / "fixtures" / "syntheseus" / str(INSTALLED_SYNTHESEUS_VERSION)
)
requires_verified_fixture_version = unittest.skipUnless(
    SYNTHESEUS_IMPORTABLE and FIXTURE_DIR.is_dir(),
    f"no committed fixtures for installed syntheseus {INSTALLED_SYNTHESEUS_VERSION!r} "
    "(verified versions: 0.7.2, 0.8.0)",
)


@requires_syntheseus
class TestSyntheseusExporter(unittest.TestCase):
    def _linear_graph(self):
        ethanol = Molecule("CCO", metadata={"is_purchasable": True})
        benzoic_acid = Molecule(
            "OC(=O)c1ccccc1",
            metadata={"is_purchasable": True, "cost": 12.5, "supplier": "TestSupplierCo"},
        )
        ethyl_benzoate = Molecule("CCOC(=O)c1ccccc1")
        step1 = SingleProductReaction(
            product=ethyl_benzoate,
            reactants=Bag([ethanol, benzoic_acid]),
            identifier="step1",
            metadata={"template": "esterification_retro", "source": "renkin-syntheseus-spike-fixture"},
        )
        graph = SynthesisGraph(step1)
        graph.assert_validity()
        return graph

    def _convergent_graph(self):
        cc = Molecule("CC")
        co_from_cc = SingleProductReaction(product=Molecule("CO"), reactants=Bag([cc]), identifier="co_from_cc")
        cs_from_co = SingleProductReaction(product=Molecule("CS"), reactants=Bag([Molecule("CO")]), identifier="cs_from_co")
        cocs_from_co_cs = SingleProductReaction(
            product=Molecule("COCS"), reactants=Bag([Molecule("CO"), Molecule("CS")]), identifier="cocs_from_co_cs"
        )
        graph = SynthesisGraph(cocs_from_co_cs)
        # Test-only: SynthesisGraph exposes no public multi-step
        # constructor, so building a >1-step graph for testing needs the
        # same private-looking `._graph.add_edge` call syntheseus's own
        # test suite uses (see syntheseus_exporter.py's module docstring
        # for why the exporter itself never touches this attribute).
        graph._graph.add_edge(cocs_from_co_cs, co_from_cc)
        graph._graph.add_edge(cocs_from_co_cs, cs_from_co)
        graph._graph.add_edge(cs_from_co, co_from_cc)
        graph.assert_validity()
        return graph

    def test_rejects_non_synthesis_graph(self):
        with self.assertRaises(TypeError):
            syn_exporter.export_syntheseus_route_v1({"not": "a graph"})

    def test_deterministic_across_two_calls(self):
        graph = self._linear_graph()
        first = syn_exporter.dumps_syntheseus_route_v1(graph)
        second = syn_exporter.dumps_syntheseus_route_v1(graph)
        self.assertEqual(first, second)

    def test_ambiguous_leaf_metadata_is_null_not_guessed(self):
        doc = syn_exporter.export_syntheseus_route_v1(self._convergent_graph())
        self.assertIn("CC", doc["molecule_metadata"])
        self.assertIsNone(doc["molecule_metadata"]["CC"]["is_purchasable"])

    @requires_verified_fixture_version
    def test_linear_fixture_matches_committed_provenance_output(self):
        expected = (FIXTURE_DIR / "linear_two_leaf_route.json").read_text(encoding="utf-8")
        actual = syn_exporter.dumps_syntheseus_route_v1(self._linear_graph())
        self.assertEqual(json.loads(actual), json.loads(expected))

    @requires_verified_fixture_version
    def test_convergent_fixture_matches_committed_provenance_output(self):
        expected = (FIXTURE_DIR / "convergent_route.json").read_text(encoding="utf-8")
        actual = syn_exporter.dumps_syntheseus_route_v1(self._convergent_graph())
        self.assertEqual(json.loads(actual), json.loads(expected))


if __name__ == "__main__":
    unittest.main()
