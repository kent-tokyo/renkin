"""Shared verification/download primitives for the asset-fetch scripts
(scripts/fetch_reranker_model.py, scripts/fetch_coverage_templates.py).

Both scripts SHA-256-verify a GitHub Release asset against one or more
committed manifests before letting a caller use it -- this module is the
part of that logic that's identical across assets (hashing, manifest
loading, download, and the fetch-then-verify-then-delete-on-failure
orchestration). What's asset-specific (which manifests exist, how many
files, what checks each file needs) stays in each script's own
`build_checks()`/`main()`.
"""

import hashlib
import json
import os
import subprocess


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


def check_asset_manifest_version(asset_manifest, version, manifest_label):
    """Raises RuntimeError if `asset_manifest` isn't pinned to `version` --
    per its own immutability policy, a new release's assets get a new
    manifest entry rather than reusing an old one. A no-op when `version`
    was left at its default (asset_manifest's own release_tag) -- this
    only ever fires when the caller passes an explicit --version that
    doesn't match what this manifest's checksums were actually issued for.

    `manifest_label` names the manifest file in the error message (e.g.
    "release_asset_manifest.json") -- callers pass their own filename so
    this one function serves every asset-fetch script's manifest."""
    pinned = asset_manifest.get("release_tag")
    if pinned != version:
        raise RuntimeError(
            f"{manifest_label} is pinned to release_tag={pinned!r}, "
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
