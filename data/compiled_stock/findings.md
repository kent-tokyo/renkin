# CompiledStockV1 local performance gate

Date: 2026-09-04
Build: `cargo build -p renkin --release --bin renkin` from `ff86d1c` plus the
uncommitted CompiledStockV1 implementation and pre-existing unrelated changes.
Method: one release binary, one process per measurement, `/usr/bin/time -lp`,
depth 0 with the target equal to the first stock entry. No network or external
service was used. Generated `.rstock` artifacts were kept under `/tmp`, not in
the repository.

## 100,397-entry union stock

Source: `data/stock_tiers/tier_100000_union_default.imported.smi`

| operation | wall time | max RSS |
|---|---:|---:|
| one-time `stock compile` | 31.36 s | 37,011,456 B |
| plain `.smi` load + depth-0 search | 30.15 s | 29,802,496 B |
| compiled `.rstock` load + depth-0 search | 0.03 s | 15,712,256 B |

The repeated-process path is approximately **1,005x faster** in this one-run
local comparison. Both searches returned the same direct-purchase route.

Artifact facts:

- molecule count: 100,397
- source SHA-256:
  `sha256:194cffe24e9750339f12db7abe5804f9918181e0c5d68b356c72536621b0e50c`
- semantic content SHA-256:
  `sha256:a40f46e996d1eb3410bdf87e30c616852c5b9e59fdc8a320ce6b55e9b4bdaa38`
- rejected rows: 0; duplicate rows: 0

## 1,000,362-entry union stock

Source: `data/stock_tiers/tier_1000000_union_default.imported.smi`

| operation | wall time | max RSS |
|---|---:|---:|
| one-time `stock compile` | 296.00 s | 334,397,440 B |
| compiled `.rstock` load + depth-0 search | 0.33 s | 132,562,944 B |

A same-build plain-load repeat was intentionally not spent after the 100k
paired result; the one-time compiler itself performs the full parse,
standardize, and canonicalize pass and bounds that work at 296 seconds here.
The earlier 817-second plain-load measurement used an older dependency state
and is not presented as a paired speed ratio for this build.

Artifact facts:

- molecule count: 1,000,362
- source SHA-256:
  `sha256:f76dda5d1126c3db1b4e0b0777b469788dc8ad8e1d264032847cb8fd6db68b76`
- semantic content SHA-256:
  `sha256:c9f89c5872afd3225255414bcbfb93659f788bd34c300e00f8041638e32ade9e`
- rejected rows: 0; duplicate rows: 0

## Gate verdict

**PASS for local candidate status.** The implementation removes the measured
per-process chemistry-normalization tax while retaining schema, normalization,
count, payload, sorting/uniqueness, and semantic-content checks. Release status
still requires the normal full workspace tests, clippy, and packaging checks.
