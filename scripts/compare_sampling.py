"""Deterministic target sampling for the Issue #66 open-source planner comparison.

Freezes a single, reproducible ordering over `data/uspto50k_test.smi` so that
the 100-target feasibility sample, the 500-target future validation sample,
and the full corpus are *prefixes of one sorted list* -- never three
independently-derived samples that merely happen to agree.

Canonicalization uses RDKit (not chematic) so the same neutral, tool-agnostic
canonicalizer used by the post-hoc validator (`compare_validation.py`) also
defines sample membership -- one canonicalizer, one place it can disagree
with a tool's own SMILES dialect, not two.

Usage:
    .venv-compare-66/bin/python scripts/compare_sampling.py \
        --corpus data/uspto50k_test.smi \
        --output-manifest data/comparison/sample_manifest.json \
        --output-list data/comparison/sample_full_sorted.jsonl
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
import stat
import tempfile
from dataclasses import dataclass, field

try:
    from rdkit import Chem, RDLogger

    RDLogger.DisableLog("rdApp.*")
    HAVE_RDKIT = True
except ImportError:  # pragma: no cover -- exercised by scripts/tests without the dep installed
    HAVE_RDKIT = False

PROTOCOL_VERSION = "renkin-issue66-sample-v1"
MAX_SAMPLE_BYTES = 64 * 1024 * 1024
MAX_SAMPLE_LINE_BYTES = 64 * 1024


def _validated_sample_file(path: str) -> os.stat_result:
    metadata = os.lstat(path)
    if stat.S_ISLNK(metadata.st_mode):
        raise ValueError(f"sample input must not be a symlink: {path!r}")
    if not stat.S_ISREG(metadata.st_mode):
        raise ValueError(f"sample input must be a regular file: {path!r}")
    if metadata.st_size > MAX_SAMPLE_BYTES:
        raise ValueError(f"sample input exceeds {MAX_SAMPLE_BYTES} bytes: {path!r}")
    return metadata


def canonical_smiles(raw_smiles: str) -> str | None:
    if not HAVE_RDKIT:
        raise RuntimeError(
            "rdkit is required (pip install -r scripts/requirements-compare-66.txt)"
        )
    mol = Chem.MolFromSmiles(raw_smiles)
    if mol is None:
        return None
    return Chem.MolToSmiles(mol, canonical=True)


def sample_key(canonical: str) -> str:
    h = hashlib.sha256()
    h.update(f"{PROTOCOL_VERSION}|".encode("utf-8"))
    h.update(canonical.encode("utf-8"))
    return h.hexdigest()


def sha256_file(path: str) -> str:
    _validated_sample_file(path)
    h = hashlib.sha256()
    total = 0
    with open(path, "rb") as handle:
        while chunk := handle.read(65536):
            total += len(chunk)
            if total > MAX_SAMPLE_BYTES:
                raise ValueError(f"sample input exceeds {MAX_SAMPLE_BYTES} bytes: {path!r}")
            h.update(chunk)
    return h.hexdigest()


def sha256_text(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


@dataclass
class CandidateLine:
    line_number: int
    raw_smiles: str


@dataclass
class SampleBuildResult:
    manifest: dict
    ordered_rows: list[dict] = field(default_factory=list)


def load_candidate_lines(corpus_path: str) -> tuple[list[CandidateLine], int, int]:
    """Returns (candidate_lines, total_lines, comment_or_blank_lines)."""
    candidates: list[CandidateLine] = []
    total_lines = 0
    comment_or_blank = 0
    _validated_sample_file(corpus_path)
    total_bytes = 0
    with open(corpus_path, "rb") as handle:
        for line_number, raw_bytes in enumerate(handle, start=1):
            total_bytes += len(raw_bytes)
            if total_bytes > MAX_SAMPLE_BYTES:
                raise ValueError(f"sample input exceeds {MAX_SAMPLE_BYTES} bytes: {corpus_path!r}")
            if len(raw_bytes) > MAX_SAMPLE_LINE_BYTES:
                raise ValueError(
                    f"sample input line exceeds {MAX_SAMPLE_LINE_BYTES} bytes: {corpus_path!r}"
                )
            raw_line = raw_bytes.decode("utf-8")
            total_lines += 1
            stripped = raw_line.strip()
            if not stripped or stripped.startswith("#"):
                comment_or_blank += 1
                continue
            # Format: "<SMILES>\t<reaction_class>" -- first whitespace token is SMILES.
            raw_smiles = stripped.split()[0]
            candidates.append(CandidateLine(line_number=line_number, raw_smiles=raw_smiles))
    return candidates, total_lines, comment_or_blank


def build_sample(corpus_path: str) -> SampleBuildResult:
    candidates, total_lines, comment_or_blank = load_candidate_lines(corpus_path)

    unparseable: list[dict] = []
    # canonical_smiles -> list of (line_number, raw_smiles), insertion order
    groups: dict[str, list[CandidateLine]] = {}
    for c in candidates:
        canon = canonical_smiles(c.raw_smiles)
        if canon is None:
            unparseable.append({"line_number": c.line_number, "raw_smiles": c.raw_smiles})
            continue
        groups.setdefault(canon, []).append(c)

    duplicate_detail = []
    kept: list[tuple[str, int]] = []  # (canonical_smiles, kept_line_number)
    for canon, entries in groups.items():
        entries_sorted = sorted(entries, key=lambda c: c.line_number)
        keeper = entries_sorted[0]
        kept.append((canon, keeper.line_number))
        if len(entries_sorted) > 1:
            duplicate_detail.append(
                {
                    "canonical_smiles": canon,
                    "raw_line_numbers": [e.line_number for e in entries_sorted],
                    "kept_line_number": keeper.line_number,
                }
            )
    duplicate_detail.sort(key=lambda d: d["kept_line_number"])

    # Single sort, by (sample_key, canonical_smiles) for a deterministic tie-break.
    keyed = [(sample_key(canon), canon, line_no) for canon, line_no in kept]
    keyed.sort(key=lambda t: (t[0], t[1]))

    ordered_rows = []
    for rank, (key, canon, line_no) in enumerate(keyed):
        ordered_rows.append(
            {
                "sample_rank": rank,
                "target_id": f"uspto50k_test#L{line_no}",
                "canonical_smiles": canon,
                "source_line_number": line_no,
                "sample_key": key,
            }
        )

    ordered_list_text = "\n".join(json.dumps(row, sort_keys=True) for row in ordered_rows) + "\n"

    manifest = {
        "protocol_version": PROTOCOL_VERSION,
        "source_file": corpus_path,
        "source_file_sha256": sha256_file(corpus_path),
        "canonicalizer": "rdkit",
        "canonicalizer_version": Chem.rdBase.rdkitVersion,
        "raw_lines_total": total_lines,
        "comment_or_blank_lines": comment_or_blank,
        "raw_candidate_lines": len(candidates),
        "unparseable_count": len(unparseable),
        "unparseable_lines": unparseable,
        "canonical_duplicate_groups": len(duplicate_detail),
        "canonical_duplicate_detail": duplicate_detail,
        "unique_canonical_targets": len(ordered_rows),
        "hash_function": 'SHA-256("renkin-issue66-sample-v1|" + canonical_smiles), lowercase hex',
        "tie_break": "ascending canonical SMILES string on sample_key collision",
        "sample_100_size": min(100, len(ordered_rows)),
        "sample_500_size": min(500, len(ordered_rows)),
        "sample_full_size": len(ordered_rows),
        "ordered_list_sha256": sha256_text(ordered_list_text),
    }
    return SampleBuildResult(manifest=manifest, ordered_rows=ordered_rows)


def load_sample(list_path: str, n: int | None = None) -> list[dict]:
    """Loads the frozen ordered list and returns the first `n` rows (or all).

    This is the ONLY way downstream code should obtain sample_100/sample_500 --
    both are prefixes of the same file, never independently recomputed.
    """
    if n is not None and (isinstance(n, bool) or n < 0):
        raise ValueError("sample size must be a non-negative integer")
    _validated_sample_file(list_path)
    rows = []
    seen_ranks: set[int] = set()
    seen_target_ids: set[str] = set()
    total_bytes = 0
    with open(list_path, "rb") as handle:
        for raw_bytes in handle:
            total_bytes += len(raw_bytes)
            if total_bytes > MAX_SAMPLE_BYTES:
                raise ValueError(f"sample input exceeds {MAX_SAMPLE_BYTES} bytes: {list_path!r}")
            if len(raw_bytes) > MAX_SAMPLE_LINE_BYTES:
                raise ValueError(
                    f"sample input line exceeds {MAX_SAMPLE_LINE_BYTES} bytes: {list_path!r}"
                )
            line = raw_bytes.decode("utf-8").strip()
            if line:
                row = json.loads(line)
                if not isinstance(row, dict):
                    raise ValueError("sample list rows must be JSON objects")
                rank = row.get("sample_rank")
                target_id = row.get("target_id")
                canonical = row.get("canonical_smiles")
                source_line = row.get("source_line_number")
                sample_key_value = row.get("sample_key")
                if isinstance(rank, bool) or not isinstance(rank, int) or rank < 0:
                    raise ValueError("sample list row has an invalid sample_rank")
                if not isinstance(target_id, str) or not target_id:
                    raise ValueError("sample list row has an invalid target_id")
                if not isinstance(canonical, str) or not canonical:
                    raise ValueError("sample list row has an invalid canonical_smiles")
                if isinstance(source_line, bool) or not isinstance(source_line, int) or source_line < 1:
                    raise ValueError("sample list row has an invalid source_line_number")
                if (
                    not isinstance(sample_key_value, str)
                    or len(sample_key_value) != 64
                    or any(character not in "0123456789abcdef" for character in sample_key_value)
                ):
                    raise ValueError("sample list row has an invalid sample_key")
                if rank in seen_ranks:
                    raise ValueError(f"sample list contains duplicate sample_rank {rank}")
                if target_id in seen_target_ids:
                    raise ValueError(f"sample list contains duplicate target_id {target_id!r}")
                seen_ranks.add(rank)
                seen_target_ids.add(target_id)
                rows.append(row)
    rows.sort(key=lambda r: r["sample_rank"])
    expected_ranks = list(range(len(rows)))
    actual_ranks = [row["sample_rank"] for row in rows]
    if actual_ranks != expected_ranks:
        raise ValueError("sample list ranks must be contiguous from zero")
    return rows if n is None else rows[:n]


def write_text_atomic(path: str, text: str) -> None:
    """Write a generated sample artifact without exposing partial output."""
    directory = os.path.dirname(os.path.abspath(path))
    basename = os.path.basename(path)
    fd, temporary_path = tempfile.mkstemp(
        prefix=f".{basename}.", suffix=".tmp", dir=directory
    )
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            handle.write(text)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary_path, path)
    except BaseException:
        try:
            os.unlink(temporary_path)
        except OSError:
            pass
        raise


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", default="data/uspto50k_test.smi")
    parser.add_argument("--output-manifest", default="data/comparison/sample_manifest.json")
    parser.add_argument("--output-list", default="data/comparison/sample_full_sorted.jsonl")
    args = parser.parse_args(argv)

    result = build_sample(args.corpus)

    write_text_atomic(
        args.output_list,
        "".join(json.dumps(row, sort_keys=True) + "\n" for row in result.ordered_rows),
    )
    write_text_atomic(
        args.output_manifest,
        json.dumps(result.manifest, indent=2, sort_keys=True) + "\n",
    )

    print(json.dumps(result.manifest, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
