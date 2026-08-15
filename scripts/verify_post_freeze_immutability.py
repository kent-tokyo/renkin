#!/usr/bin/env python3
"""Mechanically verifies the v0.24 coverage-mode formal-TEST post-freeze
immutability policy (user pre-registration, see
`data/coverage_mode_formal_test/protocol.md`): after `RELEASE_CANDIDATE_SHA`
is frozen, only formal-TEST result/report and docs-description paths may
change. Everything else -- source code, package metadata, the coverage
artifact and its manifests, the fetch script, search configuration, the
benchmark runner, and the formal protocol itself -- must be byte-identical
to what RELEASE_CANDIDATE_SHA committed.

Two independent checks, both must pass:
  1. Every file `git diff --name-only RC_SHA..REF` reports as changed must
     match an explicit allowlist of permitted paths. Default-deny: a new
     file in a path nobody thought to allowlist is a violation, not a
     silent pass.
  2. A redundant, defense-in-depth check on a specific list of paths that
     must never change: their `git rev-parse RC_SHA:<path>` tree/blob hash
     must equal `git rev-parse REF:<path>`. Catches the case where check 1
     itself has a bug (e.g. an allowlist pattern that's broader than
     intended) by verifying the actual content identity of the paths that
     matter most, not just their absence from a diff.

Usage:
    python3 scripts/verify_post_freeze_immutability.py $RELEASE_CANDIDATE_SHA
    python3 scripts/verify_post_freeze_immutability.py $RELEASE_CANDIDATE_SHA --ref HEAD
"""

from __future__ import annotations

import argparse
import fnmatch
import json
import subprocess
import sys

# Permitted to change or be newly added after the freeze. Directory
# patterns end in "/**" and also match a bare directory prefix.
ALLOWED_PATH_PATTERNS = [
    "data/coverage_mode_formal_test/results/**",
    "CHANGELOG.md",
    "README.md",
    "README_ja.md",
    "README_zh.md",
    "docs/design/coverage-mode-v0.md",
    # date-released is updated by the PASS-path release steps themselves --
    # cannot be frozen alongside the rest of the release metadata.
    "CITATION.cff",
]

# Explicitly frozen -- content-hash-checked directly, not just excluded
# from the allowlist above. Directories are checked as a whole tree.
FROZEN_PATHS = [
    "src",
    "Cargo.toml",
    "Cargo.lock",
    "pyproject.toml",
    ".github/workflows/release.yml",
    "data/phase_a5_template_scaling/templates",
    "data/coverage_mode_formal_test/protocol.md",
    "data/coverage_mode_formal_test/cohort_manifest.json",
    "scripts/select_coverage_mode_formal_test_cohort.py",
    "scripts/fetch_coverage_templates.py",
    "scripts/verify_coverage_mode_cli_matches_val_gate.py",
    "scripts/compare_renkin_adapter.py",
    "scripts/compare_run.py",
    "scripts/coverage_mode_formal_test_gate.py",
    "scripts/coverage_mode_formal_test_cohort_to_sample_list.py",
    # This checker itself -- otherwise a post-freeze edit could quietly
    # narrow its own allowlist/frozen-path list to hide a violation.
    "scripts/verify_post_freeze_immutability.py",
]


def is_allowed(path: str) -> bool:
    for pattern in ALLOWED_PATH_PATTERNS:
        if pattern.endswith("/**"):
            prefix = pattern[: -len("/**")]
            if path == prefix or path.startswith(prefix + "/"):
                return True
        elif fnmatch.fnmatchcase(path, pattern):
            return True
    return False


def run_git(args: list[str]) -> subprocess.CompletedProcess:
    return subprocess.run(["git"] + args, capture_output=True, text=True)


def git_diff_names(rc_sha: str, ref: str) -> list[str]:
    result = run_git(["diff", "--name-only", f"{rc_sha}..{ref}"])
    if result.returncode != 0:
        raise RuntimeError(f"git diff failed: {result.stderr}")
    return [line for line in result.stdout.splitlines() if line]


def git_tree_hash(rev: str, path: str) -> str | None:
    """Returns the blob/tree SHA for path at rev, or None if it doesn't
    exist at that revision (git rev-parse REV:PATH fails cleanly for a
    missing path -- treated as a hash of None, not an error, so a path
    that didn't exist at freeze time and still doesn't now is correctly
    "unchanged" (None == None), while one that got added or removed is
    correctly "changed" (None != a real hash)."""
    result = run_git(["rev-parse", f"{rev}:{path}"])
    if result.returncode != 0:
        return None
    return result.stdout.strip()


def check(rc_sha: str, ref: str = "HEAD") -> dict:
    changed = git_diff_names(rc_sha, ref)
    disallowed = sorted(p for p in changed if not is_allowed(p))

    tree_mismatches = []
    for path in FROZEN_PATHS:
        h_rc = git_tree_hash(rc_sha, path)
        h_ref = git_tree_hash(ref, path)
        if h_rc != h_ref:
            tree_mismatches.append(
                {"path": path, "rc_sha_hash": h_rc, "current_hash": h_ref}
            )

    return {
        "release_candidate_sha": rc_sha,
        "ref": ref,
        "changed_files": changed,
        "disallowed_changes": disallowed,
        "tree_hash_mismatches": tree_mismatches,
        "immutable": not disallowed and not tree_mismatches,
    }


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("release_candidate_sha")
    ap.add_argument("--ref", default="HEAD")
    args = ap.parse_args()

    result = check(args.release_candidate_sha, args.ref)
    print(json.dumps(result, indent=2, sort_keys=True))

    if not result["immutable"]:
        print(
            f"\nFAIL: post-freeze immutability violated "
            f"({len(result['disallowed_changes'])} disallowed change(s), "
            f"{len(result['tree_hash_mismatches'])} frozen-path mismatch(es))",
            file=sys.stderr,
        )
        return 1
    print(
        f"\nPASS: {args.ref} differs from {args.release_candidate_sha} only in "
        "permitted result/docs paths; every frozen path is byte-identical",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
