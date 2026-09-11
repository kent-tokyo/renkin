# Four-tool benchmark bundle

This directory defines the reproducible comparison of RENKIN, AiZynthFinder,
Syntheseus, and SynPlanner. The registry is the source of truth for the
declared arms; the protocol explains what “same conditions” means.

## Before collecting results

Run these checks from a clean checkout after all model, template, stock, and
container assets have been placed under the checkout:

```sh
python3 scripts/validate_four_tool_registry.py \
  benchmarks/four_tool/configuration_registry.json \
  --repo-root . --check-artifacts

python3 scripts/verify_four_tool_images.py \
  benchmarks/four_tool/configuration_registry.json \
  --output <run-dir>/image-identities.json
```

The image identity file is part of the run bundle. Do not replace it with an
image tag or a mutable `latest` reference.

## Staged execution

Use `--sample-size 50` for the adapter/timeout/stock gate and
`--sample-size 200` for the failure-taxonomy review. These stages are
descriptive. Use `--formal --sample-size 500` only after the registry arms are
marked `verified`, all artifacts exist, resource enforcement is verified, and
the image identity file contains immutable IDs:

```sh
python3 scripts/run_four_tool_benchmark.py \
  --formal --registry benchmarks/four_tool/configuration_registry.json \
  --image-identities <run-dir>/image-identities.json \
  --target-manifest data/comparison/sample_full_sorted.jsonl \
  --sample-size 500 --output-dir <run-dir>/rows \
  --merged-output <run-dir>/merged.jsonl \
  --report-output <run-dir>/report.json \
  --markdown-report-output <run-dir>/report.md \
  --renkin <path-to-renkin> \
  --stock data/comparison/shared_stock/shared_stock.smi \
  --synth-model-dir <path-to-localretro> \
  --synplan <path-to-synplan> \
  --synplanner-config <path-to-config> \
  --reaction-rules <path-to-rules> \
  --building-blocks <path-to-building-blocks> \
  --policy-network <path-to-policy> \
  --value-network <path-to-value>
```

The formal runner refuses candidate registries, incomplete target coverage,
missing rows, metadata mismatches, and unverified image/resource conditions.
Docker-backed rows also retain the largest sampled cgroup memory value and
the `docker_stats_sampled` measurement method; unavailable measurements remain
`not_measured`.

## Required release contents

Preserve the following under one immutable run directory:

- `configuration_registry.json` and its SHA-256;
- frozen target and shared-stock manifests and their SHA-256 values;
- `image-identities.json` and dependency lock files;
- per-arm JSONL rows and raw route artifacts;
- merged JSONL, `run_manifest.json`, JSON report, and rendered Markdown report;
- runner logs, exact commands, host/container information, and paper source.

The generated report is descriptive until the declared gate is complete. A
DOI release must preserve this directory unchanged and include the citation
metadata; a later correction gets a new versioned artifact rather than
rewriting the frozen result.
