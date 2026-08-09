import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import generate_ring_context_metadata as gen  # noqa: E402

EXTRACTED_9_SMIRKS = (
    "[C:4]-[N:5](-[C:1](=[O:2])-[c:3])-[C:6]>>O-[C:1](=[O:2])-[c:3].[C:4]-[NH:5]-[C:6]"
)


class TestClassifyIntent(unittest.TestCase):
    def test_ring_only(self):
        self.assertEqual(gen.classify_intent(5, 0, 0, 0), "ring")

    def test_non_ring_only(self):
        self.assertEqual(gen.classify_intent(0, 5, 0, 0), "non_ring")

    def test_ring_and_non_ring_is_either(self):
        self.assertEqual(gen.classify_intent(3, 2, 0, 0), "either")

    def test_ambiguous_alone_folds_into_either(self):
        self.assertEqual(gen.classify_intent(0, 0, 1, 0), "either")

    def test_ambiguous_with_non_ring_still_either(self):
        self.assertEqual(gen.classify_intent(0, 4, 2, 0), "either")

    def test_no_observations_is_unknown(self):
        self.assertEqual(gen.classify_intent(0, 0, 0, 0), "unknown")

    def test_unknown_observations_never_affect_result(self):
        self.assertEqual(gen.classify_intent(0, 3, 0, 100), "non_ring")


class TestAttributeBucket(unittest.TestCase):
    """The core fix (Issue #72 follow-up): a template's LHS pattern can match
    a real product at a site that was never the actual reaction center for
    that historical occurrence (an incidental match). `attribute_bucket`
    must exclude those before deciding ring/non_ring/ambiguous/unknown --
    the earlier version of this script counted every raw match as an
    observation, inflating counts and pushing genuinely non-ring templates
    towards `either`."""

    def test_genuine_ring_match_only(self):
        candidates = [((1, 2), True)]
        formed = {(1, 2)}
        bucket, genuine = gen.attribute_bucket(candidates, formed)
        self.assertEqual(bucket, "ring")
        self.assertEqual(genuine, {(1, 2): True})

    def test_genuine_non_ring_match_only(self):
        candidates = [((1, 2), False)]
        formed = {(1, 2)}
        bucket, genuine = gen.attribute_bucket(candidates, formed)
        self.assertEqual(bucket, "non_ring")
        self.assertEqual(genuine, {(1, 2): False})

    def test_incidental_ring_match_excluded_when_genuine_is_non_ring(self):
        # The reaction's real center (1, 2) is non-ring; the template's
        # pattern happens to also match an unrelated ring bond (3, 4)
        # elsewhere in the same product -- that reaction never touched
        # (3, 4), so it must not be counted at all, and must not push this
        # occurrence towards "either"/"ambiguous".
        candidates = [((1, 2), False), ((3, 4), True)]
        formed = {(1, 2)}
        bucket, genuine = gen.attribute_bucket(candidates, formed)
        self.assertEqual(bucket, "non_ring")
        self.assertEqual(genuine, {(1, 2): False})

    def test_all_incidental_is_unknown(self):
        candidates = [((3, 4), True), ((5, 6), False)]
        formed = {(1, 2)}
        bucket, genuine = gen.attribute_bucket(candidates, formed)
        self.assertEqual(bucket, "unknown")
        self.assertEqual(genuine, {})

    def test_duplicate_genuine_match_deduped_by_real_pair(self):
        # Two raw matches (e.g. from molecular symmetry) landing on the
        # SAME real bond must count as one observation, not two.
        candidates = [((1, 2), True), ((1, 2), True)]
        formed = {(1, 2)}
        bucket, genuine = gen.attribute_bucket(candidates, formed)
        self.assertEqual(bucket, "ring")
        self.assertEqual(genuine, {(1, 2): True})

    def test_two_distinct_genuine_centers_agreeing_not_ambiguous(self):
        candidates = [((1, 2), False), ((7, 8), False)]
        formed = {(1, 2), (7, 8)}
        bucket, genuine = gen.attribute_bucket(candidates, formed)
        self.assertEqual(bucket, "non_ring")
        self.assertEqual(len(genuine), 2)

    def test_two_distinct_genuine_centers_disagreeing_is_ambiguous(self):
        # A genuine multi-center reaction where one real reaction-center
        # bond is a ring bond and another is not -- true ambiguity, kept as
        # its own bucket rather than silently becoming "either" through the
        # ring/non_ring counters.
        candidates = [((1, 2), True), ((7, 8), False)]
        formed = {(1, 2), (7, 8)}
        bucket, genuine = gen.attribute_bucket(candidates, formed)
        self.assertEqual(bucket, "ambiguous")
        self.assertEqual(len(genuine), 2)


class TestLoadCheckedInTemplates(unittest.TestCase):
    def test_comments_and_blanks_skipped(self):
        import tempfile

        with tempfile.NamedTemporaryFile(mode="w", suffix=".smi", delete=False) as f:
            f.write("# header\n\nCC>>C.C\t10\n")
            path = f.name
        self.addCleanup(os.unlink, path)
        templates = gen.load_checked_in_templates(path)
        self.assertEqual(templates, [("CC>>C.C", 10)])


class TestTemplateIdForSmirks(unittest.TestCase):
    def test_trims_whitespace(self):
        self.assertEqual(
            gen.template_id_for_smirks("CC>>C.C"),
            gen.template_id_for_smirks("  CC>>C.C  "),
        )

    def test_matches_sha256_scheme(self):
        tid = gen.template_id_for_smirks("CC>>C.C")
        self.assertTrue(tid.startswith("smirks-sha256:"))
        self.assertEqual(len(tid), len("smirks-sha256:") + 64)


@unittest.skipUnless(
    gen.HAVE_DEPS,
    "requires rdkit/rdchiral/datasets/huggingface_hub (see scripts/requirements-ring-context.txt)",
)
class TestRdkitDependentHelpers(unittest.TestCase):
    def test_mapped_bonds_extracts_map_pairs(self):
        pairs = gen.mapped_bonds("[C:1]-[N:2]-[C:3]")
        self.assertEqual(pairs, {(1, 2), (2, 3)})

    def test_changed_bonds_for_extracted_9_is_map_1_5(self):
        self.assertEqual(gen.changed_bonds_for_template(EXTRACTED_9_SMIRKS), [(1, 5)])

    def test_real_mapped_bonds_from_parsed_molecule(self):
        mol = gen.Chem.MolFromSmiles("[CH3:1][NH2:2]")
        self.assertEqual(gen.real_mapped_bonds(mol), {(1, 2)})


if __name__ == "__main__":
    unittest.main()
