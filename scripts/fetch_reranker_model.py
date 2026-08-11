#!/usr/bin/env python3
"""Fetch the frozen LightGBM reranker model + template frequency table from
a GitHub Release asset, verifying each file with SHA-256 checks against two
committed manifests before use (Issue #101 -- "batteries-included
reranker").

model.txt is a trained artifact derived from USPTO-50k (see
data/phase3e_reranker_training/findings.md); it is not committed to this
repository and is not bundled into the crates.io/PyPI/npm packages -- it is
attached as a GitHub Release asset instead, downloaded on request, and
SHA-256 verified. This keeps a research-provenance artifact of unclear
upstream data licensing out of the MIT-licensed packages while still
letting an ordinary user get a working reranker in one command, without
running the training pipeline themselves. frequency_table.json (aggregate
per-template frequency counts, not raw training data) IS already committed
and bundled in packages -- this script re-fetches/re-verifies it too
anyway, purely for a single consistent "both files verified together"
command; it is not filling a distribution gap for that file the way it is
for model.txt.

Two manifests, two different things verified:
  - freeze_manifest.json: training-time artifact identity. model.txt's
    entry is a whole-file hash. frequency_table.json's entry is a hash of
    just the file's inner `table` data (computed before
    phase3e_export_frequency_table.py wraps it in `_purpose`/`entries`/
    `table` keys) -- read back from the file's own embedded "sha256"
    field rather than re-derived.
  - release_asset_manifest.json: release-asset identity (whole-file hash
    of exactly what was uploaded, plus the release_tag those hashes are
    valid for). Needed for frequency_table.json since freeze_manifest.json's
    entry there is not a whole-file hash and so cannot by itself catch a
    wrong/corrupted/truncated download.

`--version` defaults to release_asset_manifest.json's own `release_tag`,
NOT to Cargo.toml's crate version -- those are independently-varying
concepts (the crate version bumps on every release; the asset manifest
only gets a new release_tag when its assets are actually re-uploaded, per
its own immutability policy) and conflating them would break this script's
default invocation on every ordinary version bump, long before any new
release actually re-hosts these assets.

This script does not create, modify, or upload to any GitHub Release --
it only downloads and verifies assets that must already exist there.

Usage:
    python3 scripts/fetch_reranker_model.py
    python3 scripts/fetch_reranker_model.py --version v0.23.0 --repo kent-tokyo/renkin
"""

import argparse
import hashlib
import json
import os
import subprocess
import sys

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEFAULT_FREEZE_MANIFEST = os.path.join(
    REPO_ROOT, "data/phase3e_reranker_training/freeze_manifest.json"
)
DEFAULT_ASSET_MANIFEST = os.path.join(
    REPO_ROOT, "data/phase3e_reranker_training/release_asset_manifest.json"
)
DEFAULT_OUTPUT_DIR = os.path.join(REPO_ROOT, "data/phase3e_reranker_training")


def sha256_of(path):
    """Whole-file SHA-256."""
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return f"sha256:{h.hexdigest()}"


def embedded_json_sha256(path):
    """frequency_table.json's freeze_manifest.json entry is a hash of the
    file's inner `table` data (scripts/train_reranker.py's
    template_frequency_table_sha256), computed *before*
    phase3e_export_frequency_table.py wraps it with `_purpose`/`entries`/
    `table` keys and writes it to disk -- NOT a whole-file hash. That same
    export script embeds the identical hash in the file's own top-level
    "sha256" field for exactly this reason -- read it back rather than
    re-deriving it (which would require importing train_reranker.py's
    hashing internals here too). Raises RuntimeError (not some other
    exception type) on any malformed input, so callers only ever need to
    catch one exception type."""
    try:
        with open(path, encoding="utf-8") as f:
            data = json.load(f)
    except (json.JSONDecodeError, UnicodeDecodeError) as e:
        raise RuntimeError(f"{path} is not valid JSON: {e}") from e
    if "sha256" not in data:
        raise RuntimeError(f"{path} has no top-level \"sha256\" field to verify")
    return data["sha256"]


def load_json_manifest(path, label):
    """Load a manifest JSON file, raising RuntimeError (not a raw
    FileNotFoundError/JSONDecodeError traceback) on any failure."""
    try:
        with open(path, encoding="utf-8") as f:
            return json.load(f)
    except OSError as e:
        raise RuntimeError(f"could not read {label} at {path}: {e}") from e
    except (json.JSONDecodeError, UnicodeDecodeError) as e:
        raise RuntimeError(f"{label} at {path} is not valid JSON: {e}") from e


def manifest_lookup(manifest, dotted_key):
    try:
        node = manifest
        for part in dotted_key.split("."):
            node = node[part]
        return node
    except KeyError as e:
        raise RuntimeError(f"manifest is missing expected key {dotted_key!r}") from e


def build_checks(freeze_manifest, asset_manifest):
    """Returns {filename: [(description, expected_sha256, verifier(path)), ...]}.

    Every filename here must pass ALL of its listed checks -- see the
    module docstring for why frequency_table.json needs two different
    ones and model.txt only needs one.
    """
    try:
        frequency_table_asset_sha256 = asset_manifest["assets"]["frequency_table.json"]["sha256"]
    except KeyError as e:
        raise RuntimeError(
            "release_asset_manifest.json is missing assets.\"frequency_table.json\".sha256"
        ) from e
    return {
        "model.txt": [
            (
                "whole-file hash vs freeze_manifest.json (model_artifact.sha256)",
                manifest_lookup(freeze_manifest, "model_artifact.sha256"),
                sha256_of,
            ),
        ],
        "frequency_table.json": [
            (
                "whole-file hash vs release_asset_manifest.json (download authenticity)",
                frequency_table_asset_sha256,
                sha256_of,
            ),
            (
                "embedded inner-table hash vs freeze_manifest.json "
                "(feature_schema.template_frequency_table_sha256)",
                manifest_lookup(
                    freeze_manifest, "feature_schema.template_frequency_table_sha256"
                ),
                embedded_json_sha256,
            ),
        ],
    }


def check_asset_manifest_version(asset_manifest, version):
    """Raises RuntimeError if `asset_manifest` isn't pinned to `version` --
    per its own immutability policy, a new release's assets get a new
    manifest entry rather than reusing an old one. A no-op when `version`
    was left at its default (asset_manifest's own release_tag) -- this
    only ever fires when the caller passes an explicit --version that
    doesn't match what this manifest's checksums were actually issued for."""
    pinned = asset_manifest.get("release_tag")
    if pinned != version:
        raise RuntimeError(
            f"release_asset_manifest.json is pinned to release_tag={pinned!r}, "
            f"but --version={version!r} was requested. Per this manifest's own "
            "immutability policy, a new release's assets get a new manifest "
            "entry rather than reusing an old one -- pass --asset-manifest "
            "explicitly if you really mean to check a different release's "
            "assets against this manifest."
        )


def fetch_one(url, dest_path):
    """Download url to dest_path via curl, atomically (temp file + rename).

    Raises RuntimeError on any failure; never leaves a partial file at
    dest_path.
    """
    tmp_path = dest_path + ".part"
    try:
        result = subprocess.run(["curl", "-fsSL", "-o", tmp_path, url])
    except FileNotFoundError:
        raise RuntimeError("curl is required but was not found on PATH") from None
    if result.returncode != 0:
        if os.path.exists(tmp_path):
            os.remove(tmp_path)
        raise RuntimeError(
            f"download failed ({url}) -- curl exit code {result.returncode}. "
            "Does the release at that URL actually have this asset attached?"
        )
    os.replace(tmp_path, dest_path)


def fetch_and_verify(filename, checks, repo, version, output_dir):
    """Download `filename`, run every (description, expected_sha256,
    verifier) in `checks` against it in order, return the verified path.
    Deletes the downloaded file and raises on the first failing check --
    never returns a path that failed any check, regardless of what
    exception type a verifier raises (a corrupted download can fail in
    ways other than a clean SHA-256 mismatch, e.g. invalid UTF-8 -- the
    cleanup must not depend on which one)."""
    dest_path = os.path.join(output_dir, filename)
    url = f"https://github.com/{repo}/releases/download/{version}/{filename}"
    print(f"Fetching {filename} from {url} ...")
    fetch_one(url, dest_path)
    for description, expected_sha256, verifier in checks:
        try:
            actual_sha256 = verifier(dest_path)
        except Exception as e:
            os.remove(dest_path)
            if isinstance(e, RuntimeError):
                raise
            raise RuntimeError(
                f"{filename} failed check ({description}) with an unexpected "
                f"error: {e!r}. Deleted the downloaded file; NOT safe to use."
            ) from e
        if actual_sha256 != expected_sha256:
            os.remove(dest_path)
            raise RuntimeError(
                f"{filename} failed check ({description}) -- expected "
                f"{expected_sha256}, got {actual_sha256}. Deleted the "
                "downloaded file; NOT safe to use."
            )
        print(f"  verified {filename}: {description}")
    return dest_path


def main(argv=None):
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("--repo", default="kent-tokyo/renkin")
    parser.add_argument(
        "--version",
        default=None,
        help="Release tag, e.g. v0.22.0. Default: release_asset_manifest.json's "
        "own release_tag -- the release its checksums are actually valid for "
        "(deliberately NOT the crate's current Cargo.toml version, which is an "
        "independently-varying number).",
    )
    parser.add_argument("--freeze-manifest", default=DEFAULT_FREEZE_MANIFEST)
    parser.add_argument("--asset-manifest", default=DEFAULT_ASSET_MANIFEST)
    parser.add_argument("--output-dir", default=DEFAULT_OUTPUT_DIR)
    args = parser.parse_args(argv)

    freeze_manifest = load_json_manifest(args.freeze_manifest, "freeze_manifest.json")
    asset_manifest = load_json_manifest(args.asset_manifest, "release_asset_manifest.json")

    version = args.version or asset_manifest.get("release_tag")
    if version is None:
        parser.error(
            "release_asset_manifest.json has no release_tag and --version was "
            "not given -- pass --version explicitly"
        )
    check_asset_manifest_version(asset_manifest, version)

    os.makedirs(args.output_dir, exist_ok=True)
    checks_by_file = build_checks(freeze_manifest, asset_manifest)

    paths = {}
    for filename, checks in checks_by_file.items():
        paths[filename] = fetch_and_verify(
            filename, checks, args.repo, version, args.output_dir
        )

    print(
        "\nDone. Use with:\n"
        f"  renkin ... --reranker-model {paths['model.txt']} "
        f"--reranker-freq-table {paths['frequency_table.json']}"
    )


if __name__ == "__main__":
    try:
        main()
    except RuntimeError as e:
        print(f"ERROR: {e}", file=sys.stderr)
        sys.exit(1)
