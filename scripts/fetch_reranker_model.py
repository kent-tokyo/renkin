#!/usr/bin/env python3
"""Fetch the frozen LightGBM reranker model + template frequency table from
a GitHub Release asset, verifying each file's SHA-256 against the committed
freeze manifest before use (Issue #101 -- "batteries-included reranker").

model.txt and frequency_table.json are trained artifacts derived from
USPTO-50k (see data/phase3e_reranker_training/findings.md); they are not
committed to this repository (unlike the JSON reports next to them) and are
not bundled into the crates.io/PyPI/npm packages -- they are attached as
GitHub Release assets instead, downloaded on request, and verified against
the SHA-256 already committed in freeze_manifest.json. This keeps a
research-provenance artifact of unclear upstream data licensing out of the
MIT-licensed packages while still letting an ordinary user get a working
reranker in one command, without running the training pipeline themselves.

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
DEFAULT_MANIFEST = os.path.join(
    REPO_ROOT, "data/phase3e_reranker_training/freeze_manifest.json"
)
DEFAULT_OUTPUT_DIR = os.path.join(REPO_ROOT, "data/phase3e_reranker_training")

# filename -> dotted key into freeze_manifest.json for its expected sha256
def sha256_of(path):
    """Whole-file SHA-256 -- correct for model.txt, where
    freeze_manifest.json's model_artifact.sha256 really is a hash of the
    exact bytes on disk."""
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return f"sha256:{h.hexdigest()}"


def embedded_json_sha256(path):
    """frequency_table.json is NOT hashed as a whole file against the
    manifest -- freeze_manifest.json's feature_schema.
    template_frequency_table_sha256 is a hash of just the inner `table`
    data (scripts/train_reranker.py's template_frequency_table_sha256),
    computed *before* phase3e_export_frequency_table.py wraps it with
    `_purpose`/`entries`/`table` keys and writes it to disk. That same
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


# filename -> (dotted key into freeze_manifest.json, function(path) -> actual sha256)
ASSETS = {
    "model.txt": ("model_artifact.sha256", sha256_of),
    "frequency_table.json": (
        "feature_schema.template_frequency_table_sha256",
        embedded_json_sha256,
    ),
}


def manifest_lookup(manifest, dotted_key):
    node = manifest
    for part in dotted_key.split("."):
        node = node[part]
    return node


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


def fetch_and_verify(filename, manifest_key, verifier, manifest, repo, version, output_dir):
    """Download `filename`, verify it against `manifest` via `verifier(path)
    -> actual_sha256`, return the verified path. Deletes the downloaded file
    and raises on a mismatch (or a verifier error, e.g. corrupt JSON) --
    never returns a path that failed verification."""
    expected_sha256 = manifest_lookup(manifest, manifest_key)
    dest_path = os.path.join(output_dir, filename)
    url = f"https://github.com/{repo}/releases/download/{version}/{filename}"
    print(f"Fetching {filename} from {url} ...")
    fetch_one(url, dest_path)
    try:
        actual_sha256 = verifier(dest_path)
    except RuntimeError:
        os.remove(dest_path)
        raise
    if actual_sha256 != expected_sha256:
        os.remove(dest_path)
        raise RuntimeError(
            f"{filename} SHA-256 mismatch -- expected {expected_sha256}, got "
            f"{actual_sha256}. Deleted the downloaded file; NOT safe to use."
        )
    print(f"  verified {filename}: {actual_sha256}")
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
    parser.add_argument("--manifest", default=DEFAULT_MANIFEST)
    parser.add_argument("--output-dir", default=DEFAULT_OUTPUT_DIR)
    args = parser.parse_args(argv)

    version = args.version or version_from_cargo_toml(
        os.path.join(REPO_ROOT, "Cargo.toml")
    )
    if version is None:
        parser.error("could not read version from Cargo.toml; pass --version explicitly")

    with open(args.manifest, encoding="utf-8") as f:
        manifest = json.load(f)
    os.makedirs(args.output_dir, exist_ok=True)

    paths = {}
    for filename, (manifest_key, verifier) in ASSETS.items():
        paths[filename] = fetch_and_verify(
            filename, manifest_key, verifier, manifest, args.repo, version, args.output_dir
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
