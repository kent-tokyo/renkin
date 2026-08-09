"""Shared-stock construction: one source-of-truth building-block list fed
IDENTICALLY to both RENKIN (as its --building-blocks file) and
AiZynthFinder (as a hand-built HDF5 stock), with a guaranteed zero-diff
identity between the two.

Supersedes `scripts/compare_matched_stock.py`'s "matched-stock" conversion,
which round-tripped every compound through `smiles2stock`'s own
SMILES-reading pipeline and left a residual identity gap (9 conversion
failures, 1 stereo-layer mismatch, `roundtrip_identity_confirmed=false`) --
that gap was masked with a `roundtrip_identity_confirmed_modulo_stereo_layer`
exception, which is not an acceptable basis for a "shared stock" claim.

This script instead:

1. Parses every line of `data/building_blocks.smi` directly with RDKit
   (`Chem.MolFromSmiles`) inside the AiZynthFinder container -- never
   through `smiles2stock`'s own reader, and never on the host (the host and
   container RDKit versions disagree on at least one compound's InChIKey --
   confirmed empirically during the old matched-stock work).
2. Computes each surviving compound's InChIKey via `Chem.MolToInchiKey`.
   This is the SAME call AiZynthFinder itself makes at search time
   (`aizynthfinder.chem.mol.Molecule.inchi_key` calls
   `Chem.MolToInchiKey(self.rd_mol)` after `sanitize()` -- confirmed by
   reading the installed package's source inside the container), so there
   is no independent "conversion" step left to disagree with AiZynthFinder's
   own runtime stock lookup.
3. Writes the resulting {inchi_key} table directly to HDF5
   (`pandas.DataFrame({"inchi_key": [...]}).to_hdf(path, key="table")`),
   bypassing `smiles2stock`'s internal SMILES-reading pipeline entirely --
   confirmed empirically (toy 4-compound stock, acetanilide target) that
   AiZynthFinder's `InMemoryInchiKeyQuery` loads a hand-built HDF5 with zero
   special handling required, using exactly this format.
4. Writes the same surviving compounds' RDKit-canonical SMILES to a
   companion `.smi` file -- the SAME file RENKIN is pointed at via
   `--building-blocks`. AiZynthFinder's HDF5 is derived from this exact
   list, not a separately-sourced one.

Policy (recorded in the manifest, not implicit): shared-stock identity is
RDKit's own `MolToInchiKey` of the parsed source SMILES, stereo/isotope/
charge exactly as present in the source line -- no stripping, no
normalization beyond RDKit's default sanitization. Any compound RDKit
cannot parse, or cannot compute an InChIKey for, is EXCLUDED from the
shared stock and recorded by reason -- never silently retried, never
subject to a "modulo X" exception. The round-trip check below therefore
verifies HDF5 serialization fidelity only (read back == what was written) --
there is no separate conversion step left to verify.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import tempfile
import uuid
from pathlib import Path

from compare_sampling import sha256_file

_BUILD_SCRIPT = (
    "import json\n"
    "import pandas as pd\n"
    "from rdkit import Chem, RDLogger\n"
    "RDLogger.DisableLog('rdApp.*')\n"
    "records = []\n"
    "excluded = []\n"
    "with open('/work/source.smi') as f:\n"
    "    lines = [(i + 1, line.strip()) for i, line in enumerate(f)]\n"
    "for line_no, raw in lines:\n"
    "    if not raw or raw.startswith('#'):\n"
    "        continue\n"
    "    smi = raw.split()[0]\n"
    "    mol = Chem.MolFromSmiles(smi)\n"
    "    if mol is None:\n"
    "        excluded.append({'line_no': line_no, 'smiles': smi, 'reason': 'rdkit_unparseable'})\n"
    "        continue\n"
    "    try:\n"
    "        key = Chem.MolToInchiKey(mol) or None\n"
    "    except Exception:\n"
    "        key = None\n"
    "    if key is None:\n"
    "        excluded.append({'line_no': line_no, 'smiles': smi, 'reason': 'inchikey_computation_failed'})\n"
    "        continue\n"
    "    canon = Chem.MolToSmiles(mol)\n"
    "    records.append({'line_no': line_no, 'source_smiles': smi, 'canonical_smiles': canon, 'inchi_key': key})\n"
    "seen_keys = {}\n"
    "unique_records = []\n"
    "duplicates = []\n"
    "for r in records:\n"
    "    if r['inchi_key'] in seen_keys:\n"
    "        duplicates.append({'line_no': r['line_no'], 'smiles': r['source_smiles'],\n"
    "                            'duplicate_of_line_no': seen_keys[r['inchi_key']], 'inchi_key': r['inchi_key']})\n"
    "        continue\n"
    "    seen_keys[r['inchi_key']] = r['line_no']\n"
    "    unique_records.append(r)\n"
    "unique_records.sort(key=lambda r: r['inchi_key'])\n"
    "df = pd.DataFrame({'inchi_key': [r['inchi_key'] for r in unique_records]})\n"
    "df.to_hdf('/work/shared_stock.hdf5', key='table', mode='w')\n"
    "with open('/work/canonical_smiles.smi', 'w') as f:\n"
    "    f.write('\\n'.join(r['canonical_smiles'] for r in unique_records) + '\\n')\n"
    "summary = {'unique_records': unique_records, 'excluded': excluded, 'duplicates': duplicates,\n"
    "           'raw_line_count': len(lines)}\n"
    "with open('/work/summary.json', 'w') as f:\n"
    "    json.dump(summary, f)\n"
    "print('ok', len(unique_records), 'unique,', len(excluded), 'excluded,', len(duplicates), 'duplicate')\n"
)

_READBACK_SCRIPT = (
    "import json\n"
    "import pandas as pd\n"
    "df = pd.read_hdf('/work/shared_stock.hdf5', 'table')\n"
    "print(json.dumps(sorted(df['inchi_key'].astype(str).tolist())))\n"
)


def _run_in_container(image: str, workdir: str, script_path: str, timeout: int = 120) -> str:
    container_name = f"renkin-compare-66-sharedstock-{uuid.uuid4().hex[:12]}"
    cmd = [
        "docker",
        "run",
        "--rm",
        "--name",
        container_name,
        "--platform",
        "linux/arm64",
        "-v",
        f"{workdir}:/work",
        image,
        "python",
        script_path,
    ]
    result = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    if result.returncode != 0:
        raise RuntimeError(f"container script {script_path} failed: {result.stderr}")
    return result.stdout


def build(
    building_blocks_path: str,
    image: str,
    output_hdf5_path: str,
    output_smi_path: str,
) -> dict:
    with tempfile.TemporaryDirectory() as workdir:
        workdir_path = Path(workdir)
        source_in_workdir = workdir_path / "source.smi"
        source_in_workdir.write_text(
            Path(building_blocks_path).read_text(encoding="utf-8"), encoding="utf-8"
        )

        build_script_path = workdir_path / "build.py"
        build_script_path.write_text(_BUILD_SCRIPT, encoding="utf-8")
        _run_in_container(image, workdir, "/work/build.py")

        summary = json.loads((workdir_path / "summary.json").read_text(encoding="utf-8"))

        readback_script_path = workdir_path / "readback.py"
        readback_script_path.write_text(_READBACK_SCRIPT, encoding="utf-8")
        readback_stdout = _run_in_container(image, workdir, "/work/readback.py", timeout=60)
        readback_keys = set(json.loads(readback_stdout))

        written_keys = {r["inchi_key"] for r in summary["unique_records"]}
        missing_after_roundtrip = sorted(written_keys - readback_keys)
        extra_after_roundtrip = sorted(readback_keys - written_keys)

        Path(output_hdf5_path).write_bytes((workdir_path / "shared_stock.hdf5").read_bytes())
        Path(output_smi_path).write_bytes((workdir_path / "canonical_smiles.smi").read_bytes())

    if missing_after_roundtrip or extra_after_roundtrip:
        raise RuntimeError(
            f"shared-stock HDF5 serialization round-trip FAILED: "
            f"{len(missing_after_roundtrip)} missing, {len(extra_after_roundtrip)} extra "
            f"after write-then-read-back -- this indicates a pandas/HDF5 serialization "
            f"defect, not a conversion-fidelity issue (there is no separate conversion step)"
        )

    manifest = {
        "policy": (
            "Shared-stock identity is RDKit's Chem.MolToInchiKey() of each "
            "Chem.MolFromSmiles()-parsed source line, stereo/isotope/charge exactly "
            "as present in the source SMILES (no stripping or normalization beyond "
            "RDKit's default sanitization). The InChIKey table is written directly "
            "to HDF5 (bypassing smiles2stock's own SMILES-reading pipeline), so "
            "AiZynthFinder's runtime stock lookup (which computes the same "
            "Chem.MolToInchiKey call on its own candidate molecules) can never "
            "disagree with this stock's keys by construction. Any source line RDKit "
            "cannot parse, or cannot compute an InChIKey for, is excluded and listed "
            "under 'excluded' -- never silently retried or exempted."
        ),
        "source_file": building_blocks_path,
        "source_file_sha256": sha256_file(building_blocks_path),
        "source_raw_line_count": summary["raw_line_count"],
        "excluded_count": len(summary["excluded"]),
        "excluded": summary["excluded"],
        "duplicate_count": len(summary["duplicates"]),
        "duplicates": summary["duplicates"],
        "shared_stock_compound_count": len(summary["unique_records"]),
        "output_hdf5_path": output_hdf5_path,
        "output_hdf5_sha256": sha256_file(output_hdf5_path),
        "output_smi_path": output_smi_path,
        "output_smi_sha256": sha256_file(output_smi_path),
        "roundtrip_identity_confirmed": True,
        "roundtrip_missing_count": len(missing_after_roundtrip),
        "roundtrip_extra_count": len(extra_after_roundtrip),
    }
    return manifest


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--building-blocks", default="data/building_blocks.smi")
    parser.add_argument("--image", default="renkin-compare-66/aizynthfinder:4.4.1")
    parser.add_argument(
        "--output-hdf5", default="data/comparison/shared_stock/shared_stock.hdf5"
    )
    parser.add_argument(
        "--output-smi", default="data/comparison/shared_stock/shared_stock.smi"
    )
    parser.add_argument(
        "--output-manifest", default="data/comparison/shared_stock/shared_stock_manifest.json"
    )
    args = parser.parse_args(argv)

    Path(args.output_hdf5).parent.mkdir(parents=True, exist_ok=True)

    manifest = build(args.building_blocks, args.image, args.output_hdf5, args.output_smi)
    with open(args.output_manifest, "w", encoding="utf-8") as f:
        json.dump(manifest, f, indent=2, sort_keys=True)
        f.write("\n")
    print(json.dumps(manifest, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
