import tempfile
import unittest
from pathlib import Path

import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from compare_run import load_stock  # noqa: E402


class TestCompareRunStock(unittest.TestCase):
    def test_compiled_stock_payload_excludes_magic_and_header(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "stock.rstock"
            path.write_text(
                "RENKIN-COMPILED-STOCK-V1\n"
                '{"schema_version":1,"molecule_count":2}\n'
                "CCO\nO\n",
                encoding="utf-8",
            )
            self.assertEqual(load_stock(str(path)), ["CCO", "O"])

    def test_plain_stock_keeps_existing_line_parser(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "stock.smi"
            path.write_text("# comment\nCCO vendor-a\nO\n", encoding="utf-8")
            self.assertEqual(load_stock(str(path)), ["CCO", "O"])


if __name__ == "__main__":
    unittest.main()
