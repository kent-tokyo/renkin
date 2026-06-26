# Installation

## Python

Install from PyPI — no RDKit or any C/C++ dependency required:

```bash
pip install renkin
```

Requires Python 3.9+ and a modern platform. Pre-built wheels are provided for:

| Platform | Python | Status |
|----------|--------|--------|
| Linux x86_64 | 3.9–3.13 | ✅ GitHub Actions |
| macOS arm64 (Apple Silicon) | 3.9–3.13 | ✅ GitHub Actions |
| macOS x86_64 | 3.9–3.13 | ✅ GitHub Actions |
| Windows x86_64 | 3.9–3.13 | ✅ GitHub Actions |

If your platform isn't listed, pip will attempt to build from source (requires Rust toolchain).

## Rust

Add to `Cargo.toml`:

```toml
[dependencies]
renkin = "0.15"
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

Requires Rust 1.75+ (stable):

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
python3 -m http.server 8080  # then visit http://localhost:8080/demo/
```
