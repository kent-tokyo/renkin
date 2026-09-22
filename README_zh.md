# RENKIN

一个用 Rust 编写的逆合成规划与路线审计工具。

[English](README.md) · [日本語](README_ja.md) · [文档](https://kent-tokyo.github.io/renkin/) · [在线 Playground](https://kent-tokyo.github.io/renkin/playground/)

RENKIN 有两个用途：

- **Planner：** 从目标分子搜索到可购买起始原料的逆合成路线
- **Bridge：** 审计 RENKIN、AiZynthFinder、Syntheseus 或 SynPlanner 生成的路线

审计完全在本地运行并可复现，检查结构完整性、stock覆盖、正向重现、来源信息和审计清单。

当前版本：**v1.0.9**。公开 API 会在执行前验证输入；WASM 具有明确的搜索上限，
MCP 的数值参数和元素过滤器对非法值直接失败。标准 MCP 搜索还支持协作式
`timeout_secs` 超时预算。

审计和私有 stock policy 已拆分为确定性的可测试步骤，因此 report schema 和 policy
行为保持稳定。

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

已注册的 Phase 55 shared-stock、同一预算 TEST 完成了690个 target：RENKIN 得到
481/690 条 strict route（69.71%），AiZynthFinder 4.4.1 为32/690（4.64%）。配对 coverage
差为+65.07个百分点（95% CI +61.45 至 +68.55）。该结果只适用于固定的 cohort、asset、stock
和预算；并不代表普遍 CASP 优势或实验合成成功。peak RSS 与首次找到路线的 receipt 尚未完成，
因此也不是全 cohort 的性能比较。

[基准测试详情](https://kent-tokyo.github.io/renkin/benchmark/)提供 protocol、artifact 和主张边界。

## 开发

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

开始修改前请阅读 [`AGENTS.md`](AGENTS.md)、[`tasks/lessons.md`](tasks/lessons.md)
和 [`ROADMAP.md`](ROADMAP.md)。

重要边界：stock identity 使用标准化后的 canonical SMILES 完全匹配。
比较 manifest 会记录工具、配置、输入文件和经过检查的 worktree 状态，避免恢复运行时
静默混用不同配置。

## 许可证

MIT，见 [LICENSE](LICENSE)。
