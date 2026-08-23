"""Shared helpers for Issue #101 Phase 3 real-label generation, used by both
`generate_real_labels.py` (formal test corpus) and
`generate_train_val_labels.py` (train/val corpora). Kept in one place so the
two pipelines can never silently drift onto different canonicalization
behavior -- see Phase 3A Round 2's Blocker 2 (a text-level atom-map strip
was replaced with structural clearing in the Rust binary; duplicating this
logic in two scripts would risk one of them reverting to something unsafe).
"""

from __future__ import annotations

import hashlib
import subprocess


def sha256_of(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def canonicalize_batch(smiles_list: list[str], canonicalize_bin: str) -> list[str | None]:
    """Batch-canonicalize via `renkin-canonicalize --clear-atom-maps`.

    Always clears atom maps structurally (never a text-level regex strip --
    see the module docstring and
    `chem_env::clear_atom_maps_tests::explicit_colon_bond_with_ring_closure_digit_is_not_corrupted`
    for why). This is a safe no-op for input that has no atom maps.

    Returns None for entries the binary reports as "ERR" (unparseable).
    """
    if not smiles_list:
        return []
    inp = "\n".join(smiles_list) + "\n"
    try:
        result = subprocess.run(
            [canonicalize_bin, "--clear-atom-maps"],
            input=inp,
            capture_output=True,
            text=True,
            timeout=600,
        )
    except FileNotFoundError:
        raise RuntimeError(
            f"renkin-canonicalize binary not found at {canonicalize_bin!r}. "
            "Build with: cargo build --release --bin renkin-canonicalize"
        )
    if result.returncode != 0:
        raise RuntimeError(
            f"renkin-canonicalize failed (exit {result.returncode}):\n{result.stderr}"
        )
    lines = result.stdout.split("\n")
    if lines and lines[-1] == "":
        lines = lines[:-1]
    if len(lines) != len(smiles_list):
        raise RuntimeError(
            f"renkin-canonicalize output line count ({len(lines)}) != input count "
            f"({len(smiles_list)}) -- cannot safely align results"
        )
    return [None if line == "ERR" else line for line in lines]
