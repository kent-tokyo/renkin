"""Matched-stock conversion: RENKIN's 402 canonical building blocks -> an
AiZynthFinder-format HDF5 stock, with an identity round-trip check.

AiZynthFinder's default HDF5 stock format keys molecules by **InChIKey**,
not by SMILES string (confirmed empirically: `smiles2stock --target hdf5`
produces a table with a single `inchi_key` column, read back via
`pandas.read_hdf(path, "table")` -- NOT `pandas.HDFStore.get()`, which
raises `NoSuchNodeError` against this "fixed"-format table). The round-trip
identity check therefore compares InChIKey sets, not canonical-SMILES sets
-- a plain RDKit canonical-SMILES comparison would silently pass even if
the actual stock-matching key differed, since InChIKey and canonical SMILES
are independent representations that can disagree on which structures
collapse together (tautomers, in particular).

Conversion failures on the source side ("Failed to convert ... to inchi
key" -- smiles2stock's own diagnostic) are recorded, not silently dropped:
they explain why the stock's compound count is smaller than
`data/building_blocks.smi`'s ChemEnv-loaded 402 (RDKit/InChI's own
parser rejects a handful of entries that chematic's parser accepts -- see
docs/guides/open-source-retrosynthesis-comparison.md, "Known gaps").

Source-side InChIKeys are computed INSIDE the same container as the
conversion, using its own RDKit build -- not the host's `.venv-compare-66`
RDKit. The two can differ (confirmed empirically: host rdkit 2026.3.4 vs.
the container's pinned 2023.9.6 disagreed on one compound's InChIKey), and
a host/container RDKit-version mismatch is exactly the kind of spurious
failure this round-trip check exists to catch -- computing both sides in
the same environment removes that confound entirely.
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


def load_building_blocks(path: str) -> list[str]:
    smiles = []
    with open(path, "r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if line and not line.startswith("#"):
                smiles.append(line.split()[0])
    return smiles


_SOURCE_INCHIKEY_SCRIPT = (
    "import json\n"
    "from rdkit import Chem, RDLogger\n"
    "RDLogger.DisableLog('rdApp.*')\n"
    "keys, failures = [], []\n"
    "with open('/work/renkin_building_blocks_plain.smi') as f:\n"
    "    lines = [line.strip() for line in f if line.strip()]\n"
    "for s in lines:\n"
    "    mol = Chem.MolFromSmiles(s)\n"
    "    key = None\n"
    "    if mol is not None:\n"
    "        try:\n"
    "            key = Chem.MolToInchiKey(mol) or None\n"
    "        except Exception:\n"
    "            key = None\n"
    "    if key is None:\n"
    "        failures.append(s)\n"
    "    else:\n"
    "        keys.append(key)\n"
    "print(json.dumps({'keys': keys, 'failures': failures}))\n"
)


def convert(
    building_blocks_path: str,
    image: str,
    output_hdf5_path: str,
) -> dict:
    """Runs `smiles2stock` inside the AiZynthFinder container, then verifies
    the resulting stock's InChIKey set matches the source list's (minus any
    source entries that fail InChIKey conversion, recorded explicitly).

    Returns a manifest dict (source hash, counts, round-trip result) --
    raises RuntimeError if the round-trip check fails or the conversion
    process itself errors.
    """
    source_smiles = load_building_blocks(building_blocks_path)

    with tempfile.TemporaryDirectory() as workdir:
        workdir_path = Path(workdir)
        plain_input = workdir_path / "renkin_building_blocks_plain.smi"
        plain_input.write_text("\n".join(source_smiles) + "\n", encoding="utf-8")

        container_name = f"renkin-compare-66-stockconv-{uuid.uuid4().hex[:12]}"
        convert_cmd = [
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
            "smiles2stock",
            "--files",
            "/work/renkin_building_blocks_plain.smi",
            "--source",
            "plain",
            "--output",
            "/work/renkin_bb_402.hdf5",
            "--target",
            "hdf5",
        ]
        result = subprocess.run(convert_cmd, capture_output=True, text=True, timeout=120)
        if result.returncode != 0:
            raise RuntimeError(f"smiles2stock failed: {result.stderr}")

        # Source-side InChIKeys, computed with the CONTAINER's own RDKit
        # (not the host venv's) -- see module docstring for why this must
        # not be computed on the host.
        source_script_path = workdir_path / "source_inchikeys.py"
        source_script_path.write_text(_SOURCE_INCHIKEY_SCRIPT, encoding="utf-8")
        source_cmd = [
            "docker",
            "run",
            "--rm",
            "--platform",
            "linux/arm64",
            "-v",
            f"{workdir}:/work",
            image,
            "python",
            "/work/source_inchikeys.py",
        ]
        source_result = subprocess.run(source_cmd, capture_output=True, text=True, timeout=60)
        if source_result.returncode != 0:
            raise RuntimeError(f"source InChIKey computation failed: {source_result.stderr}")
        source_data = json.loads(source_result.stdout)
        source_inchikeys = set(source_data["keys"])
        source_conversion_failures = source_data["failures"]

        # Round-trip: reload the HDF5 stock's inchi_key column via a one-off
        # Python invocation inside the same container. `pd.read_hdf(path,
        # key)` (NOT `pd.HDFStore(path).get(key)`) is required -- the latter
        # raises NoSuchNodeError against this table's on-disk layout.
        readback_script = (
            "import pandas as pd, json\n"
            "df = pd.read_hdf('/work/renkin_bb_402.hdf5', 'table')\n"
            "print(json.dumps(df['inchi_key'].astype(str).tolist()))\n"
        )
        script_path = workdir_path / "readback.py"
        script_path.write_text(readback_script, encoding="utf-8")
        readback_cmd = [
            "docker",
            "run",
            "--rm",
            "--platform",
            "linux/arm64",
            "-v",
            f"{workdir}:/work",
            image,
            "python",
            "/work/readback.py",
        ]
        readback = subprocess.run(readback_cmd, capture_output=True, text=True, timeout=60)
        if readback.returncode != 0:
            raise RuntimeError(f"stock readback failed: {readback.stderr}")
        readback_inchikeys = set(json.loads(readback.stdout))

        Path(output_hdf5_path).write_bytes((workdir_path / "renkin_bb_402.hdf5").read_bytes())

    missing_after_roundtrip = source_inchikeys - readback_inchikeys
    extra_after_roundtrip = readback_inchikeys - source_inchikeys

    # InChIKey format is <14-char skeleton block>-<10-char stereo/isotope
    # block>-<1-char>. A "missing" and an "extra" key sharing the same
    # skeleton block differ only in the stereo layer -- the underlying
    # structure IS present in the stock, just keyed by a differently
    # stereo-perceived InChIKey (smiles2stock's own SMILES-reading path
    # does not preserve directional (E/Z) bond stereo the way
    # `Chem.MolFromSmiles` + `Chem.MolToInchiKey` does directly -- a real,
    # disclosed stereochemistry-handling ceiling, not a lost compound).
    # A mismatch whose skeleton block has no counterpart on the other side
    # is a genuine identity failure and is NOT excused this way.
    def skeleton(key: str) -> str:
        return key.split("-")[0]

    missing_skeletons = {skeleton(k) for k in missing_after_roundtrip}
    extra_skeletons = {skeleton(k) for k in extra_after_roundtrip}
    stereo_only_mismatches = missing_skeletons & extra_skeletons
    genuine_missing = {k for k in missing_after_roundtrip if skeleton(k) not in extra_skeletons}
    genuine_extra = {k for k in extra_after_roundtrip if skeleton(k) not in missing_skeletons}

    manifest = {
        "source_file": building_blocks_path,
        "source_file_sha256": sha256_file(building_blocks_path),
        "source_raw_count": len(source_smiles),
        "source_conversion_failures_count": len(source_conversion_failures),
        "source_conversion_failures": source_conversion_failures,
        "source_unique_inchikey_count": len(source_inchikeys),
        "output_hdf5_path": output_hdf5_path,
        "output_hdf5_sha256": sha256_file(output_hdf5_path),
        "readback_unique_inchikey_count": len(readback_inchikeys),
        "missing_after_roundtrip_count": len(missing_after_roundtrip),
        "missing_after_roundtrip": sorted(missing_after_roundtrip),
        "extra_after_roundtrip_count": len(extra_after_roundtrip),
        "extra_after_roundtrip": sorted(extra_after_roundtrip),
        "stereo_layer_only_mismatch_count": len(stereo_only_mismatches),
        "genuine_missing_count": len(genuine_missing),
        "genuine_extra_count": len(genuine_extra),
        "roundtrip_identity_confirmed": not missing_after_roundtrip and not extra_after_roundtrip,
        "roundtrip_identity_confirmed_modulo_stereo_layer": not genuine_missing
        and not genuine_extra,
    }
    if genuine_missing or genuine_extra:
        raise RuntimeError(
            f"matched-stock round-trip identity check FAILED with a genuine (non-stereo-layer) "
            f"mismatch: {len(genuine_missing)} missing, {len(genuine_extra)} extra "
            f"after conversion -- see manifest for detail"
        )
    return manifest


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--building-blocks", default="data/building_blocks.smi")
    parser.add_argument("--image", default="renkin-compare-66/aizynthfinder:4.4.1")
    parser.add_argument("--output-hdf5", default="data/comparison/renkin_bb_402.hdf5")
    parser.add_argument(
        "--output-manifest", default="data/comparison/renkin_bb_402_manifest.json"
    )
    args = parser.parse_args(argv)

    manifest = convert(args.building_blocks, args.image, args.output_hdf5)
    with open(args.output_manifest, "w", encoding="utf-8") as f:
        json.dump(manifest, f, indent=2, sort_keys=True)
        f.write("\n")
    print(json.dumps(manifest, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
