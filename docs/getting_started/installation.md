---
title: "Install RENKIN for Python, Rust, CLI and WebAssembly"
description: "Install RENKIN via pip, cargo, or npm, or build it from source. Covers Python wheel availability, Rust MSRV, and WASM builds."
---

# Installation

## Python

Install from PyPI — no RDKit or any C/C++ dependency required:

```bash
pip install renkin
```

Requires Python 3.8+ (`requires-python = ">=3.8"` in `pyproject.toml`). Pre-built
wheels are published for common combinations of Linux/macOS (Apple Silicon)/Windows
and recent CPython versions — see the live, authoritative list of built wheels
on the [PyPI files page](https://pypi.org/project/renkin/#files). If no wheel
matches your platform, pip will attempt to build from source (requires a Rust
toolchain — see [Building from Source](#building-from-source) below).

## Rust

Add to `Cargo.toml`:

```toml
[dependencies]
renkin = "0.21"
```

Or use cargo add:

```bash
cargo add renkin
```

## JavaScript / Node.js

```bash
npm install renkin
```

Or with yarn/pnpm:

```bash
yarn add renkin
pnpm add renkin
```

## WebAssembly (browser)

The WASM module is bundled with the npm package. For direct browser use without npm:

```html
<script type="module">
  import init, { find_routes } from 'https://unpkg.com/renkin@latest/renkin.js';
  await init('https://unpkg.com/renkin@latest/renkin_bg.wasm');
  
  const result = JSON.parse(find_routes("CC(=O)Oc1ccccc1C(=O)O", 5, 3, 0));
  console.log(result);
</script>
```

## Building from Source

Requires Rust 1.85+ (stable) — RENKIN uses `edition = "2024"`, which requires this floor:

```bash
git clone https://github.com/kent-tokyo/renkin
cd renkin
cargo build --release
```

For Python wheels (requires [maturin](https://github.com/PyO3/maturin)):

```bash
pip install maturin
maturin develop --features python
```

For WASM (requires [wasm-pack](https://rustwasm.github.io/wasm-pack/)):

```bash
wasm-pack build --target web --no-default-features
python3 -m http.server 8080  # then visit http://localhost:8080/docs/playground/
```
