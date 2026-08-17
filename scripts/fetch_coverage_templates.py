#!/usr/bin/env python3
"""Fetch the frozen v0.24 coverage-mode Stage-2 template set
(templates_2000.smi) from a GitHub Release asset, verifying it with
SHA-256 checks against two committed manifests before use -- same shape
as `scripts/fetch_reranker_model.py` for the reranker model.

templates_2000.smi is derived from USPTO-50k TRAIN (see
data/phase_a5_template_scaling/templates/coverage_templates_provenance_manifest.json);
it is committed to this repository for research/benchmark reproducibility
but is NOT bundled into the crates.io/PyPI/npm packages (see Cargo.toml's
[package].exclude) -- same reasoning as the reranker's model.txt: keeping
a research-provenance artifact of unclear upstream data licensing out of
the MIT-licensed packages, distributed instead as an opt-in GitHub
Release asset, downloaded on request and SHA-256 verified.

Two manifests, two different things verified (both happen to be
whole-file hashes of the same frozen content here, unlike the reranker's
frequency_table.json case, but checked separately anyway for the same
reason: one records derivation identity, the other records
release-asset/download identity):
  - coverage_templates_provenance_manifest.json: derivation-time
    artifact identity (source dataset, extraction command, template
    count) plus the known open USPTO-50k license-status gap. Does not
    assert a license.
  - coverage_templates_release_asset_manifest.json: release-asset
    identity (whole-file hash of exactly what was/will be uploaded,
    plus the release_tag those hashes are valid for).

`--version` defaults to coverage_templates_release_asset_manifest.json's
own `release_tag`, NOT to Cargo.toml's crate version -- same
independently-varying-concepts reasoning as fetch_reranker_model.py.

This script does not create, modify, or upload to any GitHub Release --
it only downloads and verifies an asset that must already exist there.

Usage:
    python3 scripts/fetch_coverage_templates.py
    python3 scripts/fetch_coverage_templates.py --version v0.24.0 --repo kent-tokyo/renkin
"""

import argparse
import hashlib
import json
import os
import subprocess
import sys

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEFAULT_TEMPLATES_DIR = os.path.join(REPO_ROOT, "data/phase_a5_template_scaling/templates")
DEFAULT_PROVENANCE_MANIFEST = os.path.join(
    DEFAULT_TEMPLATES_DIR, "coverage_templates_provenance_manifest.json"
)
DEFAULT_ASSET_MANIFEST = os.path.join(
    DEFAULT_TEMPLATES_DIR, "coverage_templates_release_asset_manifest.json"
)
ASSET_FILENAME = "templates_2000.smi"


def sha256_of(path):
    """Whole-file SHA-256."""
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return f"sha256:{h.hexdigest()}"


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


def build_checks(provenance_manifest, asset_manifest):
    """Returns [(description, expected_sha256, verifier(path)), ...] --
    every check must pass, matching fetch_reranker_model.py's convention,
    just for a single file here instead of two."""
    try:
        provenance_sha256 = provenance_manifest["asset_sha256"]
    except KeyError as e:
        raise RuntimeError(
            "coverage_templates_provenance_manifest.json is missing \"asset_sha256\""
        ) from e
    try:
        asset_sha256 = asset_manifest["assets"][ASSET_FILENAME]["sha256"]
    except KeyError as e:
        raise RuntimeError(
            f"coverage_templates_release_asset_manifest.json is missing "
            f"assets.{ASSET_FILENAME!r}.sha256"
        ) from e
    return [
        (
            "whole-file hash vs coverage_templates_release_asset_manifest.json (download authenticity)",
            asset_sha256,
            sha256_of,
        ),
        (
            "whole-file hash vs coverage_templates_provenance_manifest.json (derivation identity)",
            provenance_sha256,
            sha256_of,
        ),
    ]


def check_asset_manifest_version(asset_manifest, version):
    """Raises RuntimeError if `asset_manifest` isn't pinned to `version` --
    per its own immutability policy, a new release's assets get a new
    manifest entry rather than reusing an old one."""
    pinned = asset_manifest.get("release_tag")
    if pinned != version:
        raise RuntimeError(
            f"coverage_templates_release_asset_manifest.json is pinned to "
            f"release_tag={pinned!r}, but --version={version!r} was requested. "
            "Per this manifest's own immutability policy, a new release's "
            "assets get a new manifest entry rather than reusing an old one -- "
            "pass --asset-manifest explicitly if you really mean to check a "
            "different release's assets against this manifest."
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
    Deletes the downloaded file and raises on the first failing check."""
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
        help="Release tag, e.g. v0.24.0. Default: "
        "coverage_templates_release_asset_manifest.json's own release_tag "
        "(deliberately NOT the crate's current Cargo.toml version).",
    )
    parser.add_argument("--provenance-manifest", default=DEFAULT_PROVENANCE_MANIFEST)
    parser.add_argument("--asset-manifest", default=DEFAULT_ASSET_MANIFEST)
    parser.add_argument("--output-dir", default=DEFAULT_TEMPLATES_DIR)
    args = parser.parse_args(argv)

    provenance_manifest = load_json_manifest(
        args.provenance_manifest, "coverage_templates_provenance_manifest.json"
    )
    asset_manifest = load_json_manifest(
        args.asset_manifest, "coverage_templates_release_asset_manifest.json"
    )

    version = args.version or asset_manifest.get("release_tag")
    if version is None:
        parser.error(
            "coverage_templates_release_asset_manifest.json has no release_tag "
            "and --version was not given -- pass --version explicitly"
        )
    check_asset_manifest_version(asset_manifest, version)

    os.makedirs(args.output_dir, exist_ok=True)
    checks = build_checks(provenance_manifest, asset_manifest)
    path = fetch_and_verify(ASSET_FILENAME, checks, args.repo, version, args.output_dir)

    print(f"\nDone. Use with:\n  renkin ... --search-mode coverage --coverage-templates {path}")


if __name__ == "__main__":
    try:
        main()
    except RuntimeError as e:
        print(f"ERROR: {e}", file=sys.stderr)
        sys.exit(1)
