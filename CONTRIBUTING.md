# Contributing to RENKIN

Thank you for helping improve RENKIN. Keep changes small, reproducible, and
chemically conservative: a route candidate is not evidence of experimental
success.

## Before opening a pull request

```bash
git clone https://github.com/kent-tokyo/renkin.git
cd renkin
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Open a focused PR against `main`. Maintainers may make direct `main` pushes
for releases or urgent fixes; contributors should use a branch and PR.

## Contribution areas

| Area | What to include |
| --- | --- |
| Bug fix | Minimal reproducer and regression test |
| Reaction rule or template behavior | Atom-balanced rationale, targeted test, and fail-closed behavior for unsupported chemistry |
| Building blocks | A justified update to `data/building_blocks.smi`; do not add stock entries in Rust code |
| Binding or MCP change | Rust, CLI/Python/WASM/MCP compatibility coverage for the affected wire contract |
| Documentation | A runnable example and an explicit claim boundary where a result is measured |
| Benchmark tooling | Immutable input/configuration hashes and a preserved raw result path |

`DEFAULT_BUILDING_BLOCKS` in `src/lib.rs` is a compiled fallback used by
library/WASM paths. The repository's default stock data is
`data/building_blocks.smi`.

## Chemistry and evidence rules

- Stock membership is exact standardized canonical-SMILES identity, never substructure matching.
- A new disconnection must not silently discard target heavy atoms. If a balanced rule cannot be stated without inventing reagents, do not add it.
- Keep model output, audit findings, and experimental claims separate.
- Do not replace a registered benchmark result with a more favorable run; record changed assets, stock, budgets, and endpoints.

See [AGENTS.md](AGENTS.md) for implementation constraints and
[docs](https://kent-tokyo.github.io/renkin/) for public API contracts.

## Security and license

Report vulnerabilities through [GitHub Private vulnerability reporting](https://github.com/kent-tokyo/renkin/security/advisories/new), not a public issue. Contributions are licensed under [MIT](LICENSE).
