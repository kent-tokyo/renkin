#!/usr/bin/env python3
"""Fetch the frozen LightGBM reranker model + template frequency table from
a GitHub Release asset, verifying each file with SHA-256 checks against two
committed manifests before use (Issue #101 -- "batteries-included
reranker").

model.txt and frequency_table.json are trained artifacts derived from
USPTO-50k (see data/phase3e_reranker_training/findings.md); they are not
committed to this repository (unlike the JSON reports next to them) and are
not bundled into the crates.io/PyPI/npm packages -- they are attached as
GitHub Release assets instead, downloaded on request, and SHA-256 verified.
This keeps a research-provenance artifact of unclear upstream data licensing
out of the MIT-licensed packages while still letting an ordinary user get a
working reranker in one command, without running the training pipeline
themselves.

Two manifests, two different things verified:
  - freeze_manifest.json: training-time artifact identity. model.txt's
    entry is a whole-file hash. frequency_table.json's entry is a hash of
    just the file's inner `table` data (computed before
    phase3e_export_frequency_table.py wraps it in `_purpose`/`entries`/
    `table` keys) -- read back from the file's own embedded "sha256"
    field rather than re-derived.
  - release_asset_manifest.json: release-asset identity (whole-file hash
    of exactly what was uploaded). Needed for frequency_table.json since
    freeze_manifest.json's entry there is not a whole-file hash and so
    cannot by itself catch a wrong/corrupted/truncated download.

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
    hashing internals here too)."""
    try:
        with open(path, encoding="utf-8") as f:
            data = json.load(f)
    except json.JSONDecodeError as e:
        raise RuntimeError(f"{path} is not valid JSON: {e}") from e
    if "sha256" not in data:
        raise RuntimeError(f"{path} has no top-level \"sha256\" field to verify")
    return data["sha256"]


def manifest_lookup(manifest, dotted_key):
    node = manifest
    for part in dotted_key.split("."):
        node = node[part]
    return node


def build_checks(freeze_manifest, asset_manifest):
    """Returns {filename: [(description, expected_sha256, verifier(path)), ...]}.

    Every filename here must pass ALL of its listed checks -- see the
    module docstring for why frequency_table.json needs two different
    ones and model.txt only needs one.
    """
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
                asset_manifest["assets"]["frequency_table.json"]["sha256"],
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
    manifest entry rather than reusing an old one."""
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


def version_from_cargo_toml(cargo_toml_path):
    with open(cargo_toml_path, encoding="utf-8") as f:
        in_package = False
        for line in f:
            stripped = line.strip()
            if stripped == "[package]":
                in_package = True
                continue
            if stripped.startswith("[") and stripped != "[package]":
                in_package = False
            if in_package and stripped.startswith("version"):
                return "v" + stripped.split("=", 1)[1].strip().strip('"')
    return None


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
    never returns a path that failed any check."""
    dest_path = os.path.join(output_dir, filename)
    url = f"https://github.com/{repo}/releases/download/{version}/{filename}"
    print(f"Fetching {filename} from {url} ...")
    fetch_one(url, dest_path)
    for description, expected_sha256, verifier in checks:
        try:
            actual_sha256 = verifier(dest_path)
        except RuntimeError:
            os.remove(dest_path)
            raise
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
        help="Release tag, e.g. v0.23.0. Default: read from the committed "
        "Cargo.toml's [package].version, prefixed with 'v'.",
    )
    parser.add_argument("--freeze-manifest", default=DEFAULT_FREEZE_MANIFEST)
    parser.add_argument("--asset-manifest", default=DEFAULT_ASSET_MANIFEST)
    parser.add_argument("--output-dir", default=DEFAULT_OUTPUT_DIR)
    args = parser.parse_args(argv)

    version = args.version or version_from_cargo_toml(
        os.path.join(REPO_ROOT, "Cargo.toml")
    )
    if version is None:
        parser.error("could not read version from Cargo.toml; pass --version explicitly")

    with open(args.freeze_manifest, encoding="utf-8") as f:
        freeze_manifest = json.load(f)
    with open(args.asset_manifest, encoding="utf-8") as f:
        asset_manifest = json.load(f)
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
