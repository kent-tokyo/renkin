#!/usr/bin/env python3
"""Verify formal benchmark container images and print immutable identities."""

from __future__ import annotations

import argparse
import json
import subprocess
from pathlib import Path


def image_identity(image: str, timeout_s: float = 10.0) -> dict[str, str]:
    try:
        result = subprocess.run(
            ["docker", "image", "inspect", image, "--format", "{{.Id}}"],
            capture_output=True, text=True, timeout=timeout_s, check=False,
        )
    except subprocess.TimeoutExpired as exc:
        raise RuntimeError(f"docker image inspect timed out for {image!r}") from exc
    if result.returncode != 0 or not result.stdout.strip():
        raise RuntimeError(result.stderr.strip() or f"image not found: {image}")
    return {"image": image, "id": result.stdout.strip()}


def registry_images(path: str | Path) -> list[str]:
    payload = json.loads(Path(path).read_text(encoding="utf-8"))
    images = []
    for arm in payload["arms"]:
        runtime = arm.get("runtime", {})
        image = runtime.get("image") if isinstance(runtime, dict) else None
        if image and image not in images:
            images.append(image)
    return images


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("registry")
    parser.add_argument("--output", required=True)
    parser.add_argument("--timeout-s", type=float, default=10.0)
    args = parser.parse_args()
    identities = [image_identity(image, args.timeout_s) for image in registry_images(args.registry)]
    Path(args.output).write_text(json.dumps({
        "schema_version": "renkin-four-tool-image-identities/1",
        "images": identities,
    }, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(identities, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
