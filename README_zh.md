# RENKIN

一个用 Rust 编写的逆合成规划与路线审计工具。

[English](README.md) · [日本語](README_ja.md) · [文档](https://kent-tokyo.github.io/renkin/) · [在线 Playground](https://kent-tokyo.github.io/renkin/playground/)

RENKIN 有两个用途：

- **Planner：** 从目标分子搜索到可购买起始原料的逆合成路线
- **Bridge：** 审计 RENKIN、AiZynthFinder、Syntheseus 或 SynPlanner 生成的路线

审计完全在本地运行并可复现，检查结构完整性、stock覆盖、正向重现、来源信息和审计清单。

## 安装

```bash
pip install renkin
cargo add renkin
npm install renkin
```

需要 Syntheseus 支持时：

```bash
pip install 'renkin[syntheseus]'
```

## 审计路线

```python
import json
import renkin

report = json.loads(
    renkin.audit_route(open("trees.json").read(), format="aizynthfinder")
)
print(report["summary"])
```

`format`支持 `aizynthfinder`、`syntheseus`、`synplanner` 和 `renkin`。

```bash
renkin audit-route route.json --format auto --output json
```

详细说明：[审计文档](https://kent-tokyo.github.io/renkin/guides/audit-reproducibility-contract/)、
[私有 stock policy](https://kent-tokyo.github.io/renkin/guides/private-stock-policy/)、
[路线 interchange](https://kent-tokyo.github.io/renkin/guides/evidence-carrying-interchange/)

## 规划路线

```python
import json
import renkin

result = json.loads(renkin.find_routes(
    target="CC(=O)Oc1ccccc1C(=O)O",  # aspirin
    depth=5,
    max_routes=3,
))
print(len(result["routes"]))
```

CLI示例：

```bash
cargo run --release -- \
  --target "CC(=O)Oc1ccccc1C(=O)O" \
  --depth 5 --beam-width 100 --format tree
```

引擎使用 A*/AND-OR 搜索、模板索引、beam限制、stock-aware评分和正向验证。
[API文档](https://docs.rs/renkin) · [逆合成指南](https://kent-tokyo.github.io/renkin/guides/rust-retrosynthesis/)

## 组件

| 组件 | 作用 |
| --- | --- |
| `renkin` | planner、CLI、Python绑定和WASM |
| `renkin-forward` | 正向预测、enumeration、hint和validation |
| `renkin-kg` | 反应知识图谱导出 |
| `renkin-mcp` | 本地MCP服务器 |

化学处理使用 [`chematic`](https://docs.rs/chematic/)。

## MCP

```bash
cargo run --release --bin renkin-mcp
```

MCP服务器通过stdio提供路线搜索、验证、解释、约束、诊断和审计receipt。
详见 [MCP指南](https://kent-tokyo.github.io/renkin/guides/mcp/)。

## 基准测试

最新正式比较使用明确声明的shared-stock条件。RENKIN与AiZynthFinder达到相同的路线数量，
但这并不证明RENKIN在所有CASP场景中普遍优于对方。成功率、速度、stock定义、验证和路线质量应分别报告。

[基准测试详情](https://kent-tokyo.github.io/renkin/benchmark/)

## 开发

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

开始修改前请阅读 [`AGENTS.md`](AGENTS.md)、[`tasks/lessons.md`](tasks/lessons.md)
和 [`ROADMAP.md`](ROADMAP.md)。

## 许可证

MIT，见 [LICENSE](LICENSE)。
