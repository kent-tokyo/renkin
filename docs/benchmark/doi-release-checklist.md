# DOI release checklist for the four-tool benchmark

This checklist defines the publication gate for a formal benchmark artifact.
It does not publish, tag, or mint a DOI by itself.

## Required before release

- [ ] The protocol and configuration registry are frozen before inspecting
      formal results.
- [ ] All four arms pass the integrated `--formal` preflight.
- [ ] The 500-target gate and every target-coverage check pass.
- [ ] The run manifest records target, stock, registry, image-identity, and
      artifact hashes.
- [ ] Every target has one row per arm; failures and `not_measured` states have
      explicit reasons.
- [ ] The generated JSON and Markdown reports are byte-reproducible from the
      merged JSONL.
- [ ] An independent clean checkout reproduces the aggregate report.
- [ ] The paper draft contains the exact arm configurations, endpoint
      definition, statistical treatment, and limitations.
- [ ] Licenses and provenance for source code, models, templates, stock,
      checkpoints, and container dependencies are recorded.
- [ ] `CITATION.cff` and the release notes identify the benchmark protocol,
      artifact version, repository commit, and intended citation.

## Immutable release contents

Upload one versioned archive containing, at minimum:

```text
four-tool-benchmark-vX.Y.Z/
├── docs/benchmark/four-tool-protocol.md
├── docs/benchmark/four-tool-technical-paper-draft.md
├── benchmarks/four_tool/configuration_registry.json
├── target-manifest.jsonl
├── shared-stock.smi
├── image-identities.json
├── dependency-locks/
├── rows/<tool>.jsonl
├── route-artifacts/
├── merged.jsonl
├── run_manifest.json
├── report.json
├── report.md
├── commands.txt
├── logs/
└── CITATION.cff
```

The DOI landing page should point to this immutable versioned artifact and
state that the reported values are specific to the declared cohort,
configuration, stock, audit, and resource envelope. A correction must create
a new version with a new artifact hash; the frozen result must not be silently
rewritten.

## Citation and claim boundary

The DOI identifies the benchmark artifact, not an endorsement of any planner
and not a claim of universal superiority. Cite the artifact together with the
software versions and protocol revision used by a downstream study. Do not
include repository popularity indicators in the report or release metadata.
