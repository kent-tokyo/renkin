import tempfile
import unittest
from pathlib import Path

import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from compare_run import load_stock, validate_search_profile_rows  # noqa: E402
from compare_schema import PlannerComparisonRow  # noqa: E402


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

    def test_named_profile_rows_require_matching_effective_metadata(self):
        row = PlannerComparisonRow(
            target_id="target-1",
            target_smiles="CC",
            sample_rank=0,
            tool="renkin",
            tool_version="1.0.4",
            configuration_id="renkin-native-sp_balanced",
            comparison_mode="native",
            run_status="completed",
            route_found=False,
            tool_specific={
                "renkin": {
                    "search_profile": {
                        "schema_version": 1,
                        "name": "balanced",
                    }
                }
            },
        )
        validate_search_profile_rows([row], "balanced")
        with self.assertRaisesRegex(ValueError, "missing search_profile"):
            validate_search_profile_rows(
                [
                    PlannerComparisonRow(
                        target_id="target-2",
                        target_smiles="CC",
                        sample_rank=1,
                        tool="renkin",
                        tool_version="1.0.4",
                        configuration_id="renkin-native-sp_balanced",
                        comparison_mode="native",
                        run_status="completed",
                        route_found=False,
                    )
                ],
                "balanced",
            )


if __name__ == "__main__":
    unittest.main()
