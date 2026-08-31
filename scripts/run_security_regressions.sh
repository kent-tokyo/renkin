#!/usr/bin/env bash
# Run the local S5/S6 release gate without downloading project dependencies.
# cargo-deny itself must already be installed (the CI job installs it before
# invoking this script); this keeps the crates-restricted local workflow
# explicit and fail-fast.
set -euo pipefail

echo "[security] dependency policy"
cargo deny check licenses bans sources

echo "[security] MCP stdio adversarial regressions"
cargo test --test mcp_cli

echo "[security] comparison-manifest contract regressions"
python3 -m unittest scripts/tests/test_compare_manifest.py

echo "[security] comparison-sampling input regressions"
python3 -m unittest scripts/tests/test_compare_sampling.py

echo "[security] OK"
