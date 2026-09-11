import unittest
from pathlib import Path

from scripts.run_syntheseus_target import load_stock


class SyntheseusRunnerTests(unittest.TestCase):
    def test_stock_loader_ignores_comments_and_extra_columns(self):
        path = Path("data/comparison/shared_stock/shared_stock.smi")
        stock = load_stock(path)
        self.assertGreater(len(stock), 0)
        self.assertTrue(all(" " not in smiles for smiles in stock))


if __name__ == "__main__":
    unittest.main()
