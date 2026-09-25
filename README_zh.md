# RENKIN

用 Rust 编写、可在本地运行的逆合成规划与路线审计工具。

[English](README.md) · [日本語](README_ja.md) · [文档](https://kent-tokyo.github.io/renkin/) · [Playground](https://kent-tokyo.github.io/renkin/playground/)

RENKIN 有两项互补工作：

- **Planner：** 从 target SMILES 搜索至指定 building block 的逆合成路线。
- **Bridge：** 用相同的确定性检查审计 RENKIN、AiZynthFinder、Syntheseus 和 SynPlanner 的路线。

当前版本为 **v1.0.9**。提供 CLI、Rust crate、Python、MCP 和浏览器 WASM；核心化学层没有 C/C++ 依赖。
默认 planner 包含 24 条手工规则。仓库 stock 文件含 402 个化合物；WASM 和找不到该文件的
安装环境使用内置的 152 个化合物 fallback。stock 会影响结论时，请显式指定。

## 安装

```bash
pip install renkin
cargo add renkin
npm install renkin
```

## 快速开始

规划 aspirin：

```bash
renkin --target "CC(=O)Oc1ccccc1C(=O)O" --depth 5 --beam-width 100
```

在本地审计导出的路线：

```bash
renkin audit-route route.json --format auto --output json
```

Python 使用同一引擎：

```python
import json, renkin
routes = json.loads(renkin.find_routes("CC(=O)Oc1ccccc1C(=O)O", depth=5))
report = json.loads(renkin.audit_route(open("route.json").read(), format="auto"))
```

## 按用途选择入口

| 用途 | 入口 |
| --- | --- |
| 仅在浏览器中规划或审计 | [Playground](https://kent-tokyo.github.io/renkin/playground/) · [WASM API](https://kent-tokyo.github.io/renkin/api/wasm/) |
| 应用集成 | [Python](https://kent-tokyo.github.io/renkin/api/python/) · [Rust](https://docs.rs/renkin) |
| 本地 agent 工作流 | [MCP 指南](https://kent-tokyo.github.io/renkin/guides/mcp/) |
| 私有 stock 与证据审计 | [审计](https://kent-tokyo.github.io/renkin/guides/audit-reproducibility-contract/) · [policy](https://kent-tokyo.github.io/renkin/guides/private-stock-policy/) |

CLI、Python、WASM 和现代 MCP 通过 capability payload 公布各自的实际限制。除非操作者明确导出，私有输入始终保留在本地。

## 范围与基准边界

审计结果是证据，不保证实验成功、收率或安全性。stock identity 使用标准化后的 canonical SMILES 完全匹配；外部模型输出和导入路线不会在未经验证时被接受。

注册的 Phase 55 shared-stock TEST 在固定 cohort、stock、asset 与 budget 下得到：RENKIN 为 481/690（69.71%），AiZynthFinder 4.4.1 为 32/690（4.64%）条 strict route。这不能证明普遍的 CASP 优势、实验可行性或全 cohort 的速度优势。使用数字前请阅读[结果记录](https://kent-tokyo.github.io/renkin/benchmark/phase55-r2-result-20260916/)。

## 开发

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

架构、贡献方式与当前计划见 [AGENTS.md](AGENTS.md)、[CONTRIBUTING.md](CONTRIBUTING.md) 和 [ROADMAP.md](ROADMAP.md)。完整发布历史保留在 [CHANGELOG.md](CHANGELOG.md)。

## 许可证

MIT，详见 [LICENSE](LICENSE)。
