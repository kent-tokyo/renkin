import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import compare_shared_stock as ss  # noqa: E402


class TestBuildScriptConstants(unittest.TestCase):
    def test_build_script_never_silently_retries_or_exempts_failures(self):
        # The whole point of this module is that a compound RDKit can't
        # parse or key is excluded and recorded -- never given a "modulo X"
        # pass, unlike the superseded matched-stock conversion.
        self.assertIn("rdkit_unparseable", ss._BUILD_SCRIPT)
        self.assertIn("inchikey_computation_failed", ss._BUILD_SCRIPT)
        self.assertNotIn("modulo", ss._BUILD_SCRIPT)

    def test_build_script_writes_hdf5_directly_not_via_smiles2stock(self):
        self.assertIn("to_hdf", ss._BUILD_SCRIPT)
        self.assertNotIn("smiles2stock", ss._BUILD_SCRIPT)

    def test_readback_script_reads_the_table_key(self):
        self.assertIn("read_hdf", ss._READBACK_SCRIPT)
        self.assertIn("'table'", ss._READBACK_SCRIPT)


if __name__ == "__main__":
    unittest.main()
