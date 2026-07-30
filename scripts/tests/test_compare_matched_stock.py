import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import compare_matched_stock as ms  # noqa: E402


class TestLoadBuildingBlocks(unittest.TestCase):
    def test_skips_comments_and_blanks_takes_first_token(self):
        import tempfile

        with tempfile.NamedTemporaryFile(mode="w", suffix=".smi", delete=False) as f:
            f.write("# header\n\nCCO ethanol\nc1ccccc1\tbenzene\n")
            path = f.name
        self.addCleanup(os.unlink, path)

        result = ms.load_building_blocks(path)
        self.assertEqual(result, ["CCO", "c1ccccc1"])


if __name__ == "__main__":
    unittest.main()
