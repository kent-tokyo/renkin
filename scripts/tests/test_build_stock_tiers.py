import hashlib
import os
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import build_stock_tiers as tiers  # noqa: E402


class TestBuildStockTiers(unittest.TestCase):
    def _write_source(self, lines):
        f = tempfile.NamedTemporaryFile(mode="w", suffix=".smi", delete=False, encoding="utf-8")
        f.write("\n".join(lines) + "\n")
        f.close()
        self.addCleanup(os.unlink, f.name)
        return Path(f.name)

    def test_nesting_property(self):
        # 30 distinct fake compounds -- enough to build 5/10/20 nested tiers.
        lines = [f"C{'C' * i}O" for i in range(30)]
        source = self._write_source(lines)
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp)
            manifest = tiers.build_tiers(source, [5, 10, 20], out)

            tier5 = set((out / "tier_5.smi").read_text().splitlines())
            tier10 = set((out / "tier_10.smi").read_text().splitlines())
            tier20 = set((out / "tier_20.smi").read_text().splitlines())

            self.assertEqual(len(tier5), 5)
            self.assertEqual(len(tier10), 10)
            self.assertEqual(len(tier20), 20)
            self.assertTrue(tier5.issubset(tier10), "5-tier must be a subset of the 10-tier")
            self.assertTrue(tier10.issubset(tier20), "10-tier must be a subset of the 20-tier")

            for tier_manifest in manifest["tiers"]:
                self.assertEqual(tier_manifest["actual_row_count"], tier_manifest["cutoff"])

    def test_deterministic_across_runs(self):
        lines = [f"C{'C' * i}O" for i in range(15)]
        source = self._write_source(lines)
        with tempfile.TemporaryDirectory() as tmp1, tempfile.TemporaryDirectory() as tmp2:
            m1 = tiers.build_tiers(source, [10], Path(tmp1))
            m2 = tiers.build_tiers(source, [10], Path(tmp2))
            self.assertEqual(m1["tiers"][0]["output_sha256"], m2["tiers"][0]["output_sha256"])
            self.assertEqual(
                Path(tmp1, "tier_10.smi").read_text(), Path(tmp2, "tier_10.smi").read_text()
            )

    def test_blank_lines_skipped_not_selected(self):
        lines = ["CCO", "", "c1ccccc1", "", "CCN"]
        source = self._write_source(lines)
        with tempfile.TemporaryDirectory() as tmp:
            manifest = tiers.build_tiers(source, [3], Path(tmp))
            self.assertEqual(manifest["source_total_lines"], 5)
            self.assertEqual(manifest["source_blank_lines"], 2)
            self.assertEqual(manifest["source_ranked_lines"], 3)
            self.assertEqual(manifest["tiers"][0]["actual_row_count"], 3)

    def test_lexicographic_order_is_not_the_ranking(self):
        # If this ever ranked by plain file order, tier_2 would be exactly
        # the first two lines. Confirm it's a real hash-based reordering.
        lines = ["AAAA", "BBBB", "CCCC", "DDDD"]
        source = self._write_source(lines)
        with tempfile.TemporaryDirectory() as tmp:
            tiers.build_tiers(source, [2], Path(tmp))
            selected = set(Path(tmp, "tier_2.smi").read_text().splitlines())
        self.assertNotEqual(
            selected,
            {"AAAA", "BBBB"},
            "tiering must not simply be the first N lines in file order",
        )

    def test_insufficient_source_lines_raises(self):
        source = self._write_source(["CCO", "CCN"])
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises(ValueError):
                tiers.build_tiers(source, [10], Path(tmp))

    def test_rank_key_uses_domain_separator(self):
        key = tiers.rank_key("CCO")
        expected = hashlib.sha256(f"{tiers.PROTOCOL_VERSION}|CCO".encode()).hexdigest()
        self.assertEqual(key, expected)

    def test_manifest_records_rebuild_command_and_source_hash(self):
        lines = [f"C{'C' * i}O" for i in range(5)]
        source = self._write_source(lines)
        with tempfile.TemporaryDirectory() as tmp:
            manifest = tiers.build_tiers(source, [3], Path(tmp))
        self.assertEqual(manifest["source_file"], str(source))
        self.assertEqual(manifest["source_file_sha256"], tiers.sha256_file(source))
        self.assertIn("build_stock_tiers.py", manifest["rebuild_command"])
        self.assertIn("--tier 3", manifest["rebuild_command"])


if __name__ == "__main__":
    unittest.main()
