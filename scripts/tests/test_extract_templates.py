import os
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import extract_templates as et  # noqa: E402


class TestHasTopLevelDot(unittest.TestCase):
    """Issue #98: chematic-smarts 0.16.0's parse_smarts (verified against
    its actual source) never accepts a disconnected-fragment reactant --
    parse() calls parse_chain exactly once, then treats any leftover `.`
    as SmartsError::UnexpectedChar. RDKit's SMARTS parser, used as a proxy
    in is_valid_for_chematic, tolerates it -- so this needs its own,
    direct check."""

    def test_real_failing_case_from_issue_98(self):
        reactant = "[C:1]-[C:2](=[O:3])-[c:7](:[c:6]):[c:8].[OH:4]-[c:5]"
        self.assertTrue(et._has_top_level_dot(reactant))

    def test_normal_single_fragment_reactant_is_fine(self):
        self.assertFalse(et._has_top_level_dot("[O:3]=[C:2]-[OH:1]"))

    def test_two_bracket_atoms_separated_by_dot(self):
        self.assertTrue(et._has_top_level_dot("[C:1].[C:2]"))

    def test_nested_parens_without_a_dot(self):
        self.assertFalse(et._has_top_level_dot("C(C)(C)C"))

    def test_empty_string(self):
        self.assertFalse(et._has_top_level_dot(""))


class TestIsValidForChematic(unittest.TestCase):
    @unittest.skipUnless(
        et.HAVE_DEPS,
        "requires rdkit/rdchiral/datasets (see scripts/requirements-ring-context.txt)",
    )
    def test_disconnected_reactant_rejected_even_though_rdkit_would_accept_it(self):
        # RDKit's own SMARTS parser is fine with a disconnected-fragment
        # reactant -- confirming the proxy-mismatch this issue is about,
        # not just asserting our own redundant logic.
        reactant_only = "[C:1]-[C:2](=[O:3])-[c:7](:[c:6]):[c:8].[OH:4]-[c:5]"
        self.assertIsNotNone(et.Chem.MolFromSmarts(reactant_only))
        smirks = reactant_only + ">>[C:1]-[C:2](=[O:3])-[c:7](:[c:6]):[c:8].[OH:4]-[c:5]"
        self.assertFalse(et.is_valid_for_chematic(smirks))

    @unittest.skipUnless(
        et.HAVE_DEPS,
        "requires rdkit/rdchiral/datasets (see scripts/requirements-ring-context.txt)",
    )
    def test_normal_template_still_accepted(self):
        smirks = "[O:3]=[C:2]-[OH:1]>>C-[O:1]-[C:2]=[O:3]"
        self.assertTrue(et.is_valid_for_chematic(smirks))

    def test_missing_double_arrow_rejected_without_deps(self):
        # No rdkit/rdchiral needed to fail this early -- exercised
        # unconditionally so this file always has at least one real check
        # of is_valid_for_chematic, regardless of local environment.
        self.assertFalse(et.is_valid_for_chematic("not-a-valid-smirks"))


class TestResolveDatasetRevision(unittest.TestCase):
    """Issue #100: dataset revision pinning -- mirrors
    generate_ring_context_metadata.py's already-reviewed
    resolve_dataset_revision exactly. No network/heavy deps needed: the
    pinned-default and user-provided branches never touch HfApi, and the
    resolve-latest branch is exercised against a mocked HfApi below."""

    def test_default_revision_is_pinned(self):
        revision, method = et.resolve_dataset_revision("x", None, False)
        self.assertEqual(revision, et.PINNED_DATASET_REVISION)
        self.assertEqual(method, "pinned-default")

    def test_user_revision_wins(self):
        revision, method = et.resolve_dataset_revision("x", "abc123", False)
        self.assertEqual(revision, "abc123")
        self.assertEqual(method, "user-provided")

    @patch("extract_templates.HfApi", create=True)
    def test_resolve_latest_uses_hub_result(self, mock_hfapi_cls):
        # create=True: HfApi may not exist as a module attribute at all
        # when HAVE_DEPS is False (it's only bound inside the try block).
        mock_hfapi_cls.return_value.dataset_info.return_value.sha = "latest-sha-456"
        revision, method = et.resolve_dataset_revision("x", None, True)
        self.assertEqual(revision, "latest-sha-456")
        self.assertIn("resolve", method.lower())


if __name__ == "__main__":
    unittest.main()
