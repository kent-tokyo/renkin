# Contributing to RENKIN

Thank you for your interest in contributing! RENKIN is a Pure Rust retrosynthesis engine and welcomes contributions of all kinds.

## Ways to Contribute

| Type | Examples |
|---|---|
| 🧪 **Reaction rules** | Add new SMIRKS retro-rules, fix incorrect disconnections |
| 🗂️ **Building blocks** | Extend the commercial reagent database |
| 🐛 **Bug reports** | Incorrect routes, wrong SMILES output, panics |
| ✨ **Features** | New CLI flags, API improvements |
| 📖 **Documentation** | Fix typos, improve examples, add translations |
| 🧬 **Benchmarks** | New test sets, external database integration |

## Setup

```bash
git clone https://github.com/kent-tokyo/renkin.git
cd renkin
cargo build          # debug build
cargo test           # run all 57 unit tests
cargo clippy -- -D warnings   # lint
cargo fmt --check              # format check
```

## Adding a Reaction Rule

Reaction rules live in `src/chem_env.rs` inside `default_rules()`.

**SMIRKS-based rule** (most rules):
```rust
RetroRule {
    name: "your_reaction_retro".to_string(),
    smirks: "[Product:1]>>[Reactant1:2].[Reactant2:3]".to_string(),
    weight: 1.0,
    required_elements: 0,  // or use elem_mask_from_smirks() result
},
```

**Graph-based rule** (for reactions where SMIRKS leaks, e.g. Suzuki):
```rust
RetroRule {
    name: "your_graph_rule".to_string(),
    smirks: String::new(),   // empty → dispatches to apply_retro match arm
    weight: 1.0,
    required_elements: 0,
},
```
Then add a match arm in `apply_retro()` and implement the graph traversal function.

**Tips:**
- Use [Daylight SMIRKS](https://www.daylight.com/dayhtml/doc/theory/theory.smirks.html) notation
- Test your rule with `cargo test` — add a test in `src/chem_env.rs #[cfg(test)]`
- `required_elements` is a u64 bitmask: set bits for elements that MUST appear in the target

## Adding Building Blocks

The default set is `DEFAULT_BUILDING_BLOCKS` in `src/lib.rs` (canonical SMILES strings).

For large sets, users supply `--building-blocks <file>` — no code change needed.

## Running the Full Benchmark

```bash
cargo build --release
bash scripts/run_benchmark_chunks.sh \
    data/uspto50k_test.smi \
    data/templates_extracted_5000.smi \
    data/bench_chunks_my_change \
    5 100
```

## Pull Request Guidelines

1. Run `cargo fmt` and `cargo clippy -- -D warnings` before submitting
2. Add a test for new rules or features
3. Update `CHANGELOG.md` under `[Unreleased]`
4. Keep PRs focused — one feature/fix per PR

## Security

Security vulnerabilities should **not** be reported via GitHub Issues.
Use [GitHub Private vulnerability reporting](https://github.com/kent-tokyo/renkin/security/advisories/new).
See [SECURITY.md](SECURITY.md) for the full policy.

## Reporting Bugs

Use [GitHub Issues](https://github.com/kent-tokyo/renkin/issues) and include:
- SMILES of the target molecule
- Expected vs. actual output
- RENKIN version (`renkin --version` or `cargo pkgid`)

## License

By contributing, you agree that your contributions will be licensed under the [MIT License](LICENSE).
