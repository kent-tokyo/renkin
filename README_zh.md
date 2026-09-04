# RENKIN — Retrosynthesis Engine for Knowledge-Informed Navigation

> **计算机辅助合成路线设计（Computer-Aided Synthesis Planning, CASP）· 纯 Rust · WebAssembly · Python**  
> 项目名称源自日语「錬金」（れんきん，罗马字 renkin，简体中文写作"炼金"）——如同炼金术士将贱金属点化为黄金，RENKIN 将目标分子逆向拆解，还原为廉价易得的起始原料。

<p>
  <a href="https://github.com/kent-tokyo/renkin/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/kent-tokyo/renkin/actions/workflows/ci.yml/badge.svg?branch=master"></a>
  <a href="https://github.com/kent-tokyo/renkin/actions/workflows/docs.yml"><img alt="Docs" src="https://github.com/kent-tokyo/renkin/actions/workflows/docs.yml/badge.svg?branch=master"></a>
</p>

<p>
  <a href="https://crates.io/crates/renkin"><img alt="Crates.io" src="https://img.shields.io/crates/v/renkin.svg"></a>
  <a href="https://docs.rs/renkin"><img alt="docs.rs" src="https://docs.rs/renkin/badge.svg"></a>
  <a href="https://pypi.org/project/renkin/"><img alt="PyPI" src="https://img.shields.io/pypi/v/renkin.svg"></a>
  <a href="https://pypi.org/project/renkin/"><img alt="Python" src="https://img.shields.io/pypi/pyversions/renkin.svg"></a>
  <a href="https://www.npmjs.com/package/renkin"><img alt="npm" src="https://img.shields.io/npm/v/renkin.svg"></a>
  <a href="LICENSE"><img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-blue.svg"></a>
</p>

[English README](./README.md) · [日本語版 README](./README_ja.md) · [**文档**](https://kent-tokyo.github.io/renkin/) · [**在线演示 →**](https://kent-tokyo.github.io/renkin/playground/)

---

**Keep your planner. Audit every route.** 审计来自 AiZynthFinder、Syntheseus 或 RENKIN 的逆合成路径——本地运行、结果可复现，且分子结构绝不外传。

[**在浏览器中审计路径 →**](https://kent-tokyo.github.io/renkin/playground/) · [**Python 快速开始 ↓**](#审计一条路径) · [**路径规划引擎 ↓**](#快速开始)

---

## 什么是 RENKIN？

RENKIN Bridge 是一个工具无关的**路径审计器**：对来自 **AiZynthFinder**、**Syntheseus** 或 RENKIN 自身规划引擎的路径，检查结构完整性、原料库存和正向反应重现性——无论路径来自哪个工具，都走完全相同的 `pass`/`fail`/`partial` 判定流程，完全在本地完成、结果可复现（每次审计都会记录可验证的 [`audit_manifest`](https://kent-tokyo.github.io/renkin/guides/audit-reproducibility-contract/)），除非你主动要求，否则分子结构不会离开你的机器。

RENKIN 本身也是一个开源的**逆合成引擎（retrosynthesis engine）**，用于**计算机辅助合成路线设计（CASP）**，能够自动从目标分子出发，逆向发现通向廉价、可商购起始原料的化学反应路线。

完全基于 Rust 构建，使用 [`chematic`](https://docs.rs/chematic/) 化学信息学 crate——零 C/C++ 依赖，所有 crate 均启用 `#![forbid(unsafe_code)]`。同一套代码库可编译为原生 CLI、Rust 库、Python wheel（PyO3），以及完全在浏览器端运行的 WebAssembly 模块。

---

## 安装

```bash
pip install renkin          # Python
cargo add renkin            # Rust
npm install renkin          # JavaScript (browser / bundler -- see docs/api/wasm.md)
```

审计 Syntheseus 路径还需要一个可选依赖：
`pip install renkin[syntheseus]`（已针对 Syntheseus `0.7.2` 与 `0.8.0` 验证）。

---

## 在线 Playground

**[→ 立即体验](https://kent-tokyo.github.io/renkin/playground/)** — 完全运行于 WebAssembly：无需安装、无需服务器、无网络请求。

---

## 审计一条路径

带上你已经用其他工具规划好的路径——下面每条路径都会经过完全相同的审计流程：无论路径来自哪个工具，也无论你是通过 CLI、Python 还是浏览器标签页运行，都会得到相同的 `pass`/`fail`/`partial` 判定。

**AiZynthFinder**

```python
import json
import renkin

report = json.loads(
    renkin.audit_route(open("trees.json").read(), format="aizynthfinder")
)
print(report["summary"])
```

**Syntheseus**（`pip install renkin[syntheseus]`）

```python
import json
import renkin
from renkin.syntheseus_exporter import dumps_syntheseus_route_v1

route_json = dumps_syntheseus_route_v1(my_synthesis_graph)
report = json.loads(renkin.audit_route(route_json, format="syntheseus"))
print(report["summary"])
```

**在浏览器中** — 无需安装、无需上传、无需服务器：[**试用 Playground →**](https://kent-tokyo.github.io/renkin/playground/)

完整的实物输出演示：[AiZynthFinder](https://kent-tokyo.github.io/renkin/guides/aizynthfinder-audit-demo/)（英文）· [Syntheseus](https://kent-tokyo.github.io/renkin/guides/syntheseus-audit-demo/)（英文）。

---

## 快速开始

*从零开始规划一条路径——若要审计手头已有的路径，请参见上方「[审计一条路径](#审计一条路径)」。*

```python
import json
import renkin

result = json.loads(
    renkin.find_routes(
        target="CC(=O)Oc1ccccc1C(=O)O",  # 阿司匹林
        depth=5,
        max_routes=3,
    )
)

for route in result["routes"]:
    for step in route["steps"]:
        print(f"  {step['target']} → {' + '.join(step['precursors'])}  [{step['rule']}]")
```

```javascript
import init, { find_routes } from './pkg/renkin.js';
await init();
const result = JSON.parse(find_routes("CC(=O)Oc1ccccc1C(=O)O", 5, 3, 0));
```

```bash
./target/release/renkin --target "CC(=O)Oc1ccccc1C(=O)O" --depth 5 \
    --templates data/templates_extracted_5000.smi --format tree
```

```text
Target: CC(=O)Oc1ccccc1C(=O)O
Routes found: 3

Route 1  [score=1.02, depth=1]
OC(=O)c1ccccc1OC(=O)C
└── [extracted_169]
    ├── OC(=O)C  ✓ BB
    └── [OH]c1ccccc1C(=O)O  ✓ BB

Route 2  [score=1.02, depth=1]
OC(=O)c1ccccc1OC(=O)C
└── [extracted_145]
    ├── CC(=O)Cl  ✓ BB
    └── [OH]c1ccccc1C(=O)O  ✓ BB

Route 3  [score=1.03, depth=1]
OC(=O)c1ccccc1OC(=O)C
└── [extracted_238]
    ├── c1cccc(c1O)C(O)=O  ✓ BB
    └── C([OH])(=O)C  ✓ BB
```

使用 `--format mermaid` 可生成兼容 GitHub/Notion 的流程图。

[![Open In Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/github/kent-tokyo/renkin/blob/master/examples/renkin_quickstart.ipynb)

---

## 当前局限

⚠️ v1.0.0 的 4,903-target shared-stock 比较已经完成，统计优越性门槛通过；但冻结的 RENKIN 输出中有两行未通过 route-tree 完整性检查，因此正式发布门槛仍为 HOLD。v1.0.1 candidate 对这两个目标的定向复跑均已得到可解析、到达库存的路线，但更新正式结论仍需用修正版完整复跑 4,903 个目标。本仓库其他位置出现的 78.0%/95.9%/81.8%（ChEMBL）均为 validator 修复前的无效历史数值。RENKIN 不预测收率、经过实验校准的成功概率或副反应，也不会自动检索文献（`success_probability` 是基于模板频率的搜索排序分数，并非经过校准的预测值）。

---

## 为什么选择 RENKIN？

RENKIN 的设计目标是打造一套 Rust 原生的合成路线设计技术栈：

| | |
|---|---|
| **快速** | A\* / AND-OR 树搜索，结合束搜索与模板频率加权 |
| **可移植** | 原生 CLI · Python wheel · npm/WASM · 浏览器 Playground —— 同一套代码库 |
| **可解释** | 每一步都带有 `confidence`、`atom_economy`、`route_cost`、`procedure_hint` |
| **可验证** | `renkin-forward` 通过正向应用模板来验证每一步逆合成 |
| **可基准测试** | 支持 USPTO-50k、PaRoutes 风格评估、路线多样性与原子平衡检查 |
| **面向智能体** | MCP 服务器向 Claude Desktop 等 AI 智能体开放路线搜索与验证能力 |

---

## 基于约束的搜索

可根据起始原料（building block）的元素组成来限制搜索出的路线。

**默认搜索** —— 联苯（biphenyl）的全部 5 条路线：

```bash
renkin --target "c1ccc(-c2ccccc2)cc1" --templates data/templates_extracted_5000.smi --format tree
```

```text
Routes found: 5
Route 1  [score=1.00, depth=1]  c1ccccc1Br + c1c(B(O)O)cccc1
Route 2  [score=1.03, depth=1]  c1ccccc1Br + c1c(B(O)O)cccc1
Route 3  [score=1.06, depth=1]  c1cc(Cl)ccc1 + c1c(B(O)O)cccc1
Route 4  [score=1.08, depth=1]  c1(I)ccccc1  + c1c(B(O)O)cccc1
Route 5  [score=1.08, depth=1]  c1ccccc1Br  + c1(B2OC(C(C)(C)O2)(C)C)ccccc1
```

**约束搜索** —— 仅保留硼酸偶联路线，排除 Br、I 起始原料：

```bash
renkin --target "c1ccc(-c2ccccc2)cc1" --templates data/templates_extracted_5000.smi \
    --require-elements "B" --avoid-elements "Br,I" --format tree
```

```text
Routes found: 1

Route 1  [score=1.06, depth=1]
c1ccccc1-c2ccccc2
└── [extracted_398]
    ├── c1cc(Cl)ccc1  ✓ BB
    └── c1c(B(O)O)cccc1  ✓ BB
```

约束条件可自由组合，并在两个层面上生效：
- `--avoid-elements` 会在**搜索过程中就进行剪枝**——只要某个前体（precursor）起始原料含有被禁止的元素，对应的扩展节点就不会被加入搜索堆（不会产生死胡同节点）。
- 为保证正确性，最终仍会执行一次路线级别的后过滤。
- `--require-elements` 则仅作为路线级别的后过滤条件。

添加 `--verbose` 可将搜索统计信息（已展开节点数、耗时）打印到 stderr。性能计数器仅在原生构建中可用，WASM 构建中已禁用。

---

## 核心特性

| 特性 | 详情 |
|---|---|
| **纯安全 Rust（Pure Safe Rust）** | 所有 crate 均启用 `#![forbid(unsafe_code)]` —— 编译器强制保证，零 C/C++ 依赖 |
| **搜索引擎** | A\*/AND-OR 树搜索（等价于 Retro\*，启发函数可插拔替换 `MoleculeValueEstimator`/`ReactionPrior`），`--beam-width N` 限制内存占用，非 WASM 环境下 `rayon` 并行规则应用（wasm32 回退为串行） |
| **最多 5 万条反应模板** | 通过 rdchiral 从 USPTO-50k/MIT 自动提取；按出现频率加权排序（可选纯 Rust `tract-onnx` 神经网络评分器，`--scorer` 启用）；可用 `--templates` 指定自定义模板集 |
| **模板质量工具** | `renkin template stats\|validate\|dedup\|explain\|coverage` —— 频率分布、有效性、重复项、按模板查询、覆盖率 |
| **步骤元数据来源标注** | 每一步都会报告 `metadata_source`/`metadata_scope`（例如 `handcrafted_default`/`reaction_family`），从而可以机器可读地区分 `conditions`/`reaction_family` 究竟来自规则作者设定的默认值，还是有更充分依据的来源；对自动提取的模板不会填充该字段，因为没有可靠信息可填，不会凭空捏造。 |
| **稳定 template_id + evidence 元数据侧车** | 每个模板都有稳定的 `template_id`——手工规则为 `rule:<name>`，自动提取的模板为 `smirks-sha256:<hex>`（与文件顺序无关）。通过 `--template-metadata sidecar.json` 可以关联 DOI/专利、报告条件、报告收率和已知副反应警示；仅匹配到的步骤会获得 `evidence` 字段。自动收率/成功率预测和文献自动检索仍在范围之外（[#41](https://github.com/kent-tokyo/renkin/issues/41)） |
| **Ring-context安全护栏** | `--ring-context-policy conservative --ring-context-sidecar <path>` — opt-in的match-level过滤器，当extracted template的训练数据从未观察到某环开闭断裂时予以拒绝。默认为`disabled`（既有行为不变）——详见[Issue #72](https://github.com/kent-tokyo/renkin/issues/72) |
| **LightGBM candidate reranker** | `--reranker-model`/`--reranker-freq-table`（CLI）或 `reranker_model_path`/`reranker_freq_table_path`（Python）— opt-in且仅影响排序：绝不改变生成哪些候选，只改变搜索顺序，关闭时与legacy排序逐字节完全一致。Paired 100-target route-search门控结果：`route_to_configured_stock` 16→20（+4/-0）。用`python3 scripts/fetch_reranker_model.py`获取冻结模型（SHA-256验证，不随任何包分发——详见[路线图](#路线图)） |
| **Coverage mode（opt-in）** | `--search-mode coverage --coverage-templates <path>`（CLI）或 `search_mode="coverage"`、`coverage_templates_path=...`（Python）—— 仅当默认模板集找不到路线时，才自动升级到另一个更大的模板集；可通过 `--coverage-timeout-secs` 协作式取消。未启用时标准模式输出逐字节不变。用 `python3 scripts/fetch_coverage_templates.py` 获取冻结的 2,000 条模板 Stage-2 模板集（SHA-256 验证，不随任何包分发，理由同 reranker 模型——详见[路线图](#路线图)） |
| **路线评分与诊断** | 分别报告 `confidence`、`success_probability`、cost、feasibility findings、building-block diversity 与 template-proxy chemical-idea diversity；不捏造实验可行性总分——参见 [feasibility](docs/guides/route-feasibility-diagnostics.md) 与 [diversity](docs/guides/route-set-diversity.md) |
| **帕累托多目标搜索** | `--format pareto` 返回 `route_cost`、`success_probability`、`steps` 等指标上的帕累托前沿；可通过 `--objectives cost:min,success_probability:max,steps:min` 自定义目标函数 |
| **约束 DSL** | `--constraints constraints.json` —— 元素过滤、步数限制、置信度阈值、优先反应族；支持 LLM → RENKIN 的调用流程 |
| **输出格式与诊断** | `--format json\|tree\|mermaid\|explain\|compare\|compare-json\|pareto`；未找到路线时 JSON 附带 `diagnostics`（`likely_causes`/`suggestions`） |
| **`renkin-forward` 工具集** | `predict`（排序正向产物）、`validate`（正向验证每一步，支持 `--route-json` 或 stdin） |
| **`renkin-bench`** | USPTO-50k/PaRoutes 评估，`--plausibility`（正向验证综合评分）、原子平衡检查（`target_MW > Σ precursor_MW`）——详见[基准测试](#基准测试) |
| **库存管理** | `renkin stock stats\|validate\|coverage\|compile`；带完整性校验的 `.rstock` 快照避免重复解析大型库存，并分别处理供应商、价格、交期与可用性策略 |
| **MCP 服务器** | `renkin-mcp` 通过 stdio 提供 7 个工具：`find_routes`、`validate_route`、`explain_route`、`find_pareto_routes`、`plan_with_constraints`、`estimate_diversity`、`diagnose_failure`；支持 legacy `2024-11-05` 与 modern `2026-07-28` 协议 |
| **`renkin-doctor`** | 环境诊断工具 —— 模板、起始原料、Python 导入、工具版本、数据完整性 |
| **`renkin-kg`** | 反应知识图谱构建工具 —— 从路线构建分子↔反应二部图；可导出为 GraphML 或 Cypher 格式 |
| **多目标平台** | `pip install renkin`（Linux/macOS/Windows 预编译 wheel）· `npm install renkin`（约 500 KB WASM，接近原生速度） |
| **起始原料 + 立体化学** | `data/building_blocks.smi` 实际加载的去重化合物 402 种（芳基卤化物、硼酸、杂环、胺、酸、氨基酸——详见[基准测试](#基准测试)）；完整的四面体 @/@@ 与 E/Z 立体化学支持；每条路线 JSON 均含 `building_blocks` 字段（叶子起始原料 SMILES，无需手动解析） |

> **`step_confidence`/`success_probability` 既不是收率，也不是实测的成功率。**
> 它们是由模板出现频率推导出的搜索排序分数（沿路线各步骤连乘 `rule_weight / max_rule_weight`），
> 用于在搜索过程中对候选断键方式排序——
> 并非经过校准的实验成功概率，也不是预期的分离收率。
> 路线级别的实验收率/成功率报告目前尚未实现。

---

## 流水线示例

```bash
# 使用真实商业价格进行路线成本评分
renkin -t "Cc1ccc(-c2ccccc2)cc1" --bb-prices data/prices.csv --format json

# 正向验证 —— 直接将 find_routes 的输出通过管道传入
renkin -t "CC(=O)Oc1ccccc1C(=O)O" --format json | renkin-forward validate

# 使用键中心索引加速模板检索（约提速 24%）
renkin -t "c1ccc(NC(=O)c2ccccc2)cc1" --templates data/templates_extracted_5000.smi --bond-index
```

---

## 基准测试

USPTO-50k 测试集（4,907 个分子，全量评估）：

> **评估定义**：若 `find_routes` 在 depth=5、beam=100 的限制下，能返回至少一条叶子前体全部属于起始原料集合的路线，则该分子被视为*已解决（solved）*。**不会**与 USPTO-50k 给出的真实试剂（ground-truth reactants）进行比对——任何可商购原料可达的路线均计入。

### v1.0.0 正式 shared-stock 比较（4,903 个配对目标）

| Arm | Primary route-to-shared-stock 成功数 | Rate |
|---|---:|---:|
| RENKIN v1.0.0 | 577 / 4,903 | 11.77% |
| AiZynthFinder 4.4.1 | 200 / 4,903 | 4.08% |

RENKIN 相对 AiZynthFinder 的配对差值为 **+7.689 个百分点**，paired-bootstrap
95% CI 为 **[+6.812, +8.566]**，通过预注册统计门槛。但冻结的 RENKIN 输出中有
两行缺少可解析的 normalized route tree，因此正式发布门槛仍为 **HOLD**。
v1.0.1 candidate 的修复使这两个目标的定向复跑均可解析并到达库存；只有完整重跑
4,903 个目标后才能更新冻结结论。这是 shared-stock 路线端点结果，并非实验收率或
普遍 CASP 优越性声明。[协议与状态](docs/benchmark/formal-v1.0-competitor-comparison.md) ·
[冻结报告](data/comparison/formal_v1.0/formal_report.md)

### 修正后基线（commit `e20dc8c`，2026-07-22）

| 公开名称 | 内部指标 | 数值 |
|---|---|---|
| 搜索命中库存率 | `raw_solved_rate` | **20.09%**（986/4,907） |
| 原子平衡过滤后比率 | `atom_balanced_solved_rate` | **15.41%**（756/4,907）—— 是搜索命中库存率的子集 |
| 当前验证器确认比率 | `provenance_validated_solved_rate` | **0.88%**（43/4,907）—— 是原子平衡过滤后比率的子集 |

402 种起始原料（`data/building_blocks.smi` 中实际加载的去重化合物数，详见下文）、5,000 条提取模板、28 条人工编写规则，depth=5，beam=100。这三个比率是针对同一批 4,907 个目标分子的嵌套序列，并非相互独立的数字，也都不是经过实验验证的合成成功率，或经人类化学家审核的路线准确率。`provenance_validated_solved_rate` 并非实测的化学准确率，也不是经证明的正确率下限——它只统计当前验证器能够正面确认的路线，判定为"无效"的结果中有未知比例可能是验证器的假阴性，而非真实的化学或路线错误（该比例尚未测量）。完整方法说明、按规则细分数据与复现命令参见 [`tasks/phase31_final_remeasurement_run.md`](https://github.com/kent-tokyo/renkin/blob/master/tasks/phase31_final_remeasurement_run.md) · [完整基准测试详情 →](https://kent-tokyo.github.io/renkin/benchmark/)。

### 历史演进数据（修正前，已判定无效——参见上方提示）

⚠️ 本小节中的数值（单次搜索 78.0%、cascade 95.9%、ChEMBL OOD 81.8%）均早于 31.11/31.12 号修复，已被判定无效，且尚未重新测量。仅为保持历史连续性而保留——请勿作为当前性能引用。

> **评估说明**：以下所有数值均基于标准的 USPTO-50k 训练/测试集划分（同一语料库）。模板从训练集中提取，在测试集上评估。这些数值反映的是 USPTO-50k 域内的表现；分布外（OOD）泛化能力则通过 ChEMBL 已批准药物单独评估（**81.8%**，409/500，同样尚未重新测量）。

| 配置 | 已解决 | 比率 | 起始原料数 | 模板数 | depth | beam | ms/mol |
|---|---|---|---|---|---|---|---|
| v0.1.0 初始版本 | 366/4907 | 7.5% | 463 | 31 | 3 | 50 | — |
| + 自动模板（top-300） | 1363/4907 | 27.8% | 463 | 222 | 3 | 50 | — |
| + depth=5，top-500 模板 | 2315/4907 | 47.2% | 463 | 314 | 5 | 50 | — |
| + beam=100 | 2688/4907 | 54.8%* | 463 | 314 | 5 | 100 | — |
| + Phase A（模板频率加权） | 3540/4907 | 72.1%† | 463 | 314 | 5 | 100 | — |
| + 5,000 条模板，480 种起始原料 | 3826/4907 | 78.0% | 480 | 5,000 | 5 | 100 | 2,775 |
| Phase A 无限制（beam=0） | 3832/4907 | 78.1% | 480 | 5,000 | 5 | 0 | — |
| Phase B（神经网络评分器，tract-onnx） | 3826/4907 | 78.0% | 480 | 5,000 | 5 | 100 | 3,394 |
| **+ 二芳基砜（diaryl sulfone）规则，509 种起始原料** | **3826/4907** | **78.0%** | **509** | **5,000** | **5** | **100** | **≈2,800** |
| Cascade（第二阶段：对未解决目标使用 depth=7，beam=300） | 4705/4907 | **95.9%** | 509 | 5,000 | 7 | 300 | — |

\* 29/50 个分块，使用当时的旧版二进制  
† 全部 50/50 个分块完成 —— **72.1%**（3,540/4,907）已确认  
本历史表格中的起始原料数（463/480/509）均为各时间点的原始记录值——属于历史文档遗留数值，未针对 `ChemEnv::bb_count()` 重新核验。上方"修正后基线"章节使用的是当前 `data/building_blocks.smi` 实际加载的数量（402）。

*注：LocalRetro（53.4%）与 GLG（58.0%）报告的是单步 top-1 预测准确率——这是另一种指标，不能直接比较。*

> **基准测试范围说明**：这里使用 USPTO-50k 仅作为*标准化的健全性基准（sanity benchmark）*，并不能证明其在真实世界中具有广泛的合成能力。该语料库覆盖的反应空间较窄（主要是医药合成中常见的 C–C、C–N 成键反应），在 USPTO 中样本稀少的反应类型系统性地覆盖不足。在 ChEMBL 已批准药物上的分布外（OOD）表现（**81.8%**，409/500，修正前数值，尚未重新测量）曾提示规则集的泛化能力超出了测试语料库范围，但这两个历史数值都不应被解读为对任意目标分子路线质量的保证。

### PaRoutes 兼容性

RENKIN 兼容 [PaRoutes](https://github.com/AstraZeneca/PaRoutes) 多步基准测试。下载其库存化合物与目标分子后即可直接传入：

```bash
renkin-bench \
  --input paroutes_n1_targets.smi \
  --building-blocks paroutes_stock.smi \
  --templates data/templates_extracted_5000.smi \
  --depth 5 --beam-width 100
```

JSON 输出除标准的 solved/success_rate 指标外，还包含 `avg_nodes_expanded`、`avg_confidence`、`avg_convergency` 以及 `avg_success_prob`（Retro-prob 风格）。

---

## 竞品对比

⚠️ 下表中 RENKIN 一行使用的是修正后的 `raw_solved_rate`（20.09%，参见本文档开头的提示）——早期版本此表中引用的 cascade 95.9% 数值已被判定无效且尚未重新测量，本表未收录该数值。

| 工具 | 语言 | 许可证 | WASM | 零依赖 | 算法 | 模板来源 | 库存 |
|---|---|---|---|---|---|---|---|
| **ASKCOS** | Python | CC BY-NC | 否 | 否（需 Docker，64 GB） | MCTS + A\* | USPTO（ML） | ZINC |
| **AiZynthFinder** | Python | MIT | 否 | 否（需 conda + 模型） | MCTS | USPTO（ML，约 5 万条） | eMolecules（约 600 万） |
| **SYNTHIA** | 闭源 | 专有 | 否 | 否 | SMARTS + AND/OR | 人工整理 | Sigma-Aldrich |
| **IBM RXN** | 闭源 | 云端 SaaS | 否 | 否 | Transformer | USPTO | — |
| **Retro\*** | Python | MIT | 否 | 否（已停止维护） | A\* + AND/OR | USPTO（ML） | eMolecules |
| **★ RENKIN** | **Rust** | **MIT** | **是** | **是** | **A\* + AND/OR** | 人工整理 + rdchiral（默认 5 千条；通过 `--templates` 可达 5 万条） | 402+ |

`raw_solved_rate` 是 RENKIN 现有指标中与上述其他规划工具已公开的路线搜索成功率最接近的一个，但这些数字并不能直接比较——各系统的库存规模、模板库、目标集合、搜索预算与路线质量检查方式均不相同，本表并不能说明 RENKIN 优于或劣于其他方案。

**RENKIN 的目标**：仅依靠人工整理的规则与自动提取的 SMIRKS 模板，就达到最先进水平的准确率——无需 GPU、无需训练数据、没有黑箱。在 RENKIN 当前的基准测试设置下（修正后基线，commit `e20dc8c`，2026-07-22），单次搜索的 `raw_solved_rate` 达到 **20.09%**（986/4,907）——完整的嵌套指标系列，以及为何更严格的 `provenance_validated_solved_rate`（0.88%）并非 RENKIN 实测或有明确边界的正确率，详见上方基准测试章节。RENKIN 可运行于任何地方：浏览器、CLI、Python——只需一次 `cargo build`。

> ⚠️ 上表汇总的是各工具公开但条件不同的数据，不能横向比较。上文另列的
> v1.0.0 正式实验在冻结的共享库存、目标集合和端点下直接比较了 RENKIN 与
> AiZynthFinder；该结论仅适用于那份协议，不能扩展为普遍 CASP 优越性声明。

---

## MCP 服务器

`renkin-mcp` 将逆合成能力以 MCP 工具的形式对外提供，AI 智能体（Claude 等）可直接调用。

**配置方法** —— 添加到 `claude_desktop_config.json`：

```json
{
  "mcpServers": {
    "renkin": { "command": "/path/to/renkin-mcp" }
  }
}
```

**工具列表**（7 个）：

| 工具 | 说明 |
|---|---|
| `find_routes` | 逆合成：SMILES → 带评分的路线 |
| `validate_route` | 对逆合成路线进行正向验证 |
| `explain_route` | 输出每条路线的人类可读优缺点分析 |
| `find_pareto_routes` | 帕累托前沿多目标路线搜索 |
| `plan_with_constraints` | 基于约束 DSL 的规划（元素过滤、步数限制、置信度阈值） |
| `estimate_diversity` | 路线多样性与覆盖率指标 |
| `diagnose_failure` | 以结构化结果说明搜索未找到路线的原因 |

`find_routes` 也支持 `search_mode: "coverage"`，此时必须提供
`coverage_templates` 路径。它先运行 Stage 1，仅当没有找到路线时才升级到 Stage 2，
并在响应中报告所选阶段、超时状态和各阶段耗时。

服务器会自动检测工作目录下的 `data/building_blocks.smi` 与可选的本地生成文件 `data/templates_extracted_5000.smi`；若未找到，则回退到内置的 `DEFAULT_BUILDING_BLOCKS` / `default_rules()` 默认值（按 `ChemEnv::bb_count()` 计算为 152 种去重起始原料和 23 条手工规则）。

```bash
cargo build --release
# 二进制文件: target/release/renkin-mcp
```

---

## 架构

### 工作区范围

```
┌──────────────────────────────────────────────────────────────────┐
│ renkin workspace（本仓库）                                        │
│                                                                  │
│  renkin（逆合成）                  renkin-forward                  │
│  ──────────────────────           ─────────────────────────────  │
│  target → precursors              reactants → products           │
│  A* / AND-OR 搜索                 基于模板的正向反应              │
│  路线评分与约束                   （用于验证逆合成路线）           │
│        │                                    │                    │
│        └──────────────────┬─────────────────┘                    │
│                           ▼                                      │
│               chematic（分子表示、SMILES、子结构匹配、            │
│               反应 SMARTS）                                      │
└──────────────────────────────────────────────────────────────────┘
```

### 内部数据流（renkin crate）

```
目标 SMILES
     │
     ▼
┌─────────────────────────┐
│     chem_env.rs         │  ← chematic 封装层
│  - SMILES 解析          │     canonical-SMILES FxHashSet 起始原料查找（O(1)）
│  - 20 条内置规则 + 通过 --templates 最多可加载 5 万条  │     片段净化处理 + 环泄漏过滤
│  - 起始原料匹配检查     │     apply_retro 记忆化缓存
└────────────┬────────────┘
             │  par_iter（rayon / WASM 下为串行）
             ▼
┌─────────────────────────┐
│      search.rs          │  ← A* / AND-OR 树搜索
│  - 优先级队列            │     SA Score 启发函数 + 记忆化
│  - 关闭列表              │     束搜索（SmallVec 前沿队列）
│  - Arc<PathNode> 路径    │     每个子节点 O(1) 路径共享
└────────────┬────────────┘
             │
             ▼
┌─────────────────────────┐
│      score.rs           │  ← 启发式/代价函数
│  - SA Score（chematic）  │     h = Σ(1 + 0.5·(sa−1)/9)
│  - 分子量步骤代价        │     g = Σ(1 + total_mw/2000)
└────────────┬────────────┘
             │
             ▼
┌─────────────────────────┐   （可选）
│      scorer.rs          │  ← Phase B：神经网络模板评分器
│  - tract-onnx            │     纯 Rust ONNX 推理
│  - --scorer 标志         │     针对具体分子的模板排序
└────────────┬────────────┘
             │
             ▼
  JSON  ←  CLI / Python / WASM
```

---

## 项目结构

```
renkin/                          ← Cargo workspace 根目录
├── Cargo.toml
├── src/                         ← renkin crate（逆合成）
│   ├── lib.rs                   # 公共库
│   ├── main.rs                  # CLI 二进制（--templates、--scorer、--constraints、--objectives 等参数）
│   ├── bin/benchmark.rs         # renkin-bench 二进制（--plausibility 参数）
│   ├── bin/doctor.rs            # renkin-doctor 环境诊断二进制
│   ├── bin/fp.rs                # renkin-fp ECFP4 指纹（nn-scoring 特性）
│   ├── bin/mcp.rs               # renkin-mcp MCP 服务器入口（7 个工具）
│   ├── mcp/                     # JSON-RPC、双协议协商、stdio 与工具处理
│   ├── chem_env.rs              # 逆合成规则 + 起始原料查找 + 模板加载器
│   ├── score.rs                 # SA Score 启发函数 + 步骤代价
│   ├── search.rs                # A* / AND-OR 树搜索引擎 + 束剪枝
│   ├── scorer.rs                # Phase B：tract-onnx 神经网络模板评分器
│   ├── candidate.rs             # 一步候选提案（离线重排序基础设施，尚未接入搜索）
│   ├── pool_export.rs           # 候选池 JSONL + 可复现性 manifest 导出
│   ├── python.rs                # PyO3 绑定（--features python）
│   └── wasm.rs                  # wasm-bindgen 绑定（cfg = wasm32）
├── crates/                      ← 同级 crate
│   ├── renkin-forward/          # 正向反应预测（reactants → products）
│   └── renkin-kg/               # 反应知识图谱构建工具（GraphML / Cypher 导出）
├── data/
│   ├── building_blocks.smi              # 402 种人工整理的市售起始原料（实际加载去重后的数量）
│   ├── templates_extracted_500.smi      # Git 管理的 500 条自动提取 SMIRKS 模板
│   ├── benchmark_targets.smi            # 内部基准测试集合
│   └── bench_chunks/                    # USPTO-50k 按分块的结果
├── scripts/
│   ├── extract_templates.py         # rdchiral 模板提取流水线
│   ├── run_benchmark_chunks.sh      # 可续跑的分块基准测试脚本
│   ├── train_reranker.py            # 候选重排序器训练/评估（开发工具，仅限离线）
│   └── tests/                       # train_reranker.py 的 unittest 套件
├── docs/                # MkDocs 源文件 → kent-tokyo.github.io/renkin/
└── mkdocs.yml
```

---

## 路线图

全部发布历史（按时间顺序）：[`CHANGELOG.md`](CHANGELOG.md)（英文）。
本节仅追踪当前的重点项目与后续计划——更早的已发布功能见下方「更早的里程碑」。

### 近期完成

- [x] 让重排序器真正可用：Python 接口（`find_routes()` 的 `reranker_model_path`/`reranker_freq_table_path`）与 batteries-included 模型分发（`scripts/fetch_reranker_model.py`，从 v0.22.0 GitHub Release 的正式 asset 下载并做 SHA-256 校验）（[#101](https://github.com/kent-tokyo/renkin/issues/101)，v0.23.0 发布）——v0.22.0 已证明重排序器有效，v0.23.0 是可用性/分发层面的解锁，而非新的精度提升主张
- [x] 将 LightGBM 候选重排序器离线训练、通过门控，并接入 route search（[#101](https://github.com/kent-tokyo/renkin/issues/101) Task 35，CLI 已在 v0.22.0 发布）— 基于真实 USPTO-50k 标签训练 LambdaMART 模型，通过 VAL screening gate（top1 +11.7pp、MRR +11.3pp、top10 +9.3pp，均经 bootstrap CI 确认），冻结模型后对该 frozen 模型仅执行一次正式的 4,903-target TEST 评估并通过（top1 +12.7pp、MRR +11.9pp、top10 +9.1pp——与 VAL 幅度一致，无过拟合迹象），随后作为 ordering-only 的 rank bonus 接入 `find_routes`，并通过 paired 100-target route-search 门控确认：`route_to_configured_stock` 16→20/100（+4/-0）。详见上方 Key Features 表
- [x] 500-target规模的RENKIN vs AiZynthFinder正式比较（[#66](https://github.com/kent-tokyo/renkin/issues/66)）— 在固定500-target样本、共享393化合物库存、各工具自身配置下，RENKIN Conservative的`route_to_shared_stock`比AiZynthFinder高9.8个百分点（73/500对24/500，95% CI [7.0, 12.8]，exact McNemar p≈1.9e-11）。这是该协议下的配对结果，并非泛化的搜索能力优越性主张；原生配置的库存不一致，不能直接比较。
- [x] extracted template的Ring-context安全护栏（[#72](https://github.com/kent-tokyo/renkin/issues/72)/[#242](https://github.com/kent-tokyo/renkin/pull/242)）— opt-in的`--ring-context-policy`/`--ring-context-sidecar`，检测训练数据中从未观察到环结合的环开闭断裂被extracted template误用的情况。默认仍为`disabled`（既有行为不变）
- [x] `atom_economy`不再隐式钳制为100%（[#79](https://github.com/kent-tokyo/renkin/issues/79)）— 当路线的呈现前体集合无法解释目标全部质量时，新增`atom_economy_status`字段（`normal`/`above_expected_range`/`not_evaluable`）明确报告
- [x] Coverage mode（`--search-mode coverage`，[#101](https://github.com/kent-tokyo/renkin/issues/101)，v0.24.0 发布）—— opt-in 的 Stage-1/Stage-2 模板数量升级机制，用于应对下方的 candidate-generation coverage gap。已通过一次性 500-target 规模的 formal-TEST 确认（`data/coverage_mode_formal_test/protocol_v2.md`）：coverage +6.0pp、net gain +30、regression 0、reranker failure 0、Stage-2 timeout 率 0.25%——均达到预注册阈值。已发布范围详见上方核心特性表

### 进行中

- [ ] Candidate-generation coverage gap —— formal TEST 语料库中 33.0%（1,618/4,903）的目标 in-pool 候选数为零，这是重排序在原理上无法解决的天花板；template-diversity-scaling 已确认是有效机制（Phase A.5/B.2，见上方 coverage mode），higher-level-template 研究方向尚未启动
- [ ] 面向 5 万条模板集合的模板检索索引（元素位掩码 + 键中心预筛选）
- [ ] 校准过的路线置信度（将 `success_probability` 映射到经验已解决率）

### 下一步

- [ ] 基于图的规则扩展 — 磺酰胺 / 氨基甲酸酯 / 脲的裂解（每个反应族独立提交一个 PR，各自附带基准测试增量）
- [ ] 面向库存的规划（按价格/危险性/可获得性重新排序）

<details>
<summary>更早的里程碑</summary>

以下百分比数字是各里程碑达成时的历史数据，并非当前性能——部分早于[当前局限](#当前局限)中提到的validator精度修复，已失效；修正后的历史数字见[基准测试](#基准测试)。

- [x] `renkin-forward enumerate` — 从单一已知反应物加明确partner库进行有界、template引导的正向枚举（[#64](https://github.com/kent-tokyo/renkin/issues/64)）
- [x] `renkin-forward hints` — 无需partner输入的检索提示（匹配的template slot、缺失partner的SMARTS、结合变化），不预测具体产物（[#64](https://github.com/kent-tokyo/renkin/issues/64) phase 2）
- [x] `apply_retro`/`run_reactants` 性能回归修复 — `chematic` 从窄范围的 git-pin 修复迁移到已发布的 `0.8.0`（上游 automorphism-orbit-pruned canonicalization，[chematic#193](https://github.com/kent-tokyo/chematic/pull/193)）；正确性零变化
- [x] `renkin-forward` CLI 强化 — 带版本号的 `ForwardPredictionReport`、确定性候选ID/合并/来源信息、与 reactant 顺序无关的匹配（最多 3 个反应物）、严格的 CLI/route-JSON 校验
- [x] 受 RETROSPECT 启发的离线候选重排序基础设施 — proposal/selection 分离、feature schema v1、manifest v2、leakage-safe 的 train/val/test 划分、7 个确定性 baseline arm + 训练模型 arm、paired bootstrap + 离线门控工具（[#59](https://github.com/kent-tokyo/renkin/pull/59)）
- [x] 确定性的 ORD（Open Reaction Database）evidence 导入 — 离线的 `renkin evidence match`（exact-set 批量 template matcher）+ `scripts/ord_evidence_audit.py`（audit/converter）转换为 `schema_version: 2` 附加文件。无网络访问、无 fuzzy matching，存疑/来源不明的记录不会被猜测，而是记录在 audit report 中并注明排除原因（[#41](https://github.com/kent-tokyo/renkin/issues/41) phase 3A）
- [x] 稳定的 `template_id`（`rule:<name>` / `smirks-sha256:<hex>`）+ `--template-metadata` evidence 附加文件 + `renkin template ids`（[#41](https://github.com/kent-tokyo/renkin/issues/41) phase 1）
- [x] 针对特定底物的 `examples`（`schema_version: 2`）——按每个步骤解析为「精确底物匹配」或「同模板但底物不同」，在 `--format explain` 中展示，并在 JSON 中以 `match_kind` 字段体现（[#41](https://github.com/kent-tokyo/renkin/issues/41) phase 2）
- [x] `renkin-bench cascade` — 多阶段搜索（先用较快的默认参数搜索，未解决的困难目标再用更深的参数重跑）；只有未解决的目标才会进入后续阶段
- [x] `renkin-bench --failure-taxonomy` — 按失败原因对未解决目标分类（束宽限制 / 深度限制 / 模板缺口 / 库存接近命中）
- [x] 基于图的酯裂解 — 无 BFS 泄漏的 `R-C(=O)-O-R' → RCOOH + R'OH`
- [x] `--top-templates N` — 按频率排序过滤：仅使用出现频率最高的前 N 条模板，以提升速度/降低噪声
- [x] `raw / validated / practical` 三档已解决率指标（`--plausibility --practical-max-steps N`）
- [x] `SearchStats` 中新增 Retro 缓存命中率 + `--verbose`
- [x] 路线成本评分 —— `route_cost` 字段 + `--bb-prices path.csv` / `--stock stock.csv`
- [x] Cargo workspace —— `crates/renkin-forward/` + `crates/renkin-kg/`
- [x] `renkin-forward predict` / `validate` —— 正向预测 + 路线验证（对 stdin 管道友好）
- [x] `renkin-doctor` —— 环境诊断工具（模板、起始原料、Python、二进制文件）
- [x] 失败诊断 —— 未找到路线时的输出包含 `likely_causes` + `suggestions` 的 JSON 区块
- [x] `--format explain|compare|compare-json` —— 人类可读与表格形式的路线输出
- [x] `renkin stock stats|validate|coverage|compile` —— 库存检查与带完整性校验的 `.rstock` 快照
- [x] 帕累托多目标搜索 —— `--format pareto`、`--objectives`、`find_pareto_routes` MCP 工具
- [x] 约束 DSL —— `--constraints JSON`、`plan_with_constraints` MCP 工具
- [x] `renkin template stats|validate|dedup|explain|coverage` —— 模板质量工具
- [x] `renkin-kg` —— 反应知识图谱（分子↔反应二部图，GraphML/Cypher 导出）
- [x] MCP 服务器扩展至 7 个工具，并支持 legacy `2024-11-05` / modern `2026-07-28` stdio 协议
- [x] 核心搜索引擎基础 —— SMIRKS 逆反应规则 + 片段净化处理，A\*/AND-OR 树搜索（含 closed list + 退化路线过滤），SA Score 启发函数 + 束搜索，`rayon` 并行规则应用（wasm32 回退为串行），FxHashMap/SmallVec 束搜索前沿/SA Score 记忆化/`Arc<PathNode>` 路径共享等性能优化
- [x] 多目标分发 —— Python 绑定（PyO3 + maturin，`pip install renkin`）、WASM 构建（`npm install renkin`）、已发布至 crates.io/PyPI/npm 并配备 GitHub Actions CI/CD、WASM 浏览器 Playground + 国际化（EN/JA/ZH）
- [x] 基准测试 CLI（`renkin-bench`）+ USPTO-50k 评估、`--format tree|mermaid` 可视化、MkDocs 文档站点 + GitHub Pages Playground
- [x] 基于图的联芳基裂解 · O(1) canonical-SMILES 起始原料索引
- [x] 四面体立体化学 @/@@ + E/Z 双键立体化学
- [x] 通过 `--scorer` 标志接入神经网络模板评分器（tract-onnx，纯 Rust ONNX）
- [x] 基于约束的搜索：`--avoid-elements`、`--require-elements` + `--verbose` 搜索统计信息输出
- [x] `#![forbid(unsafe_code)]` —— 编译器强制保证的纯安全 Rust

</details>

---

## 引用

若您在学术工作中使用 RENKIN，请引用 [`CITATION.cff`](CITATION.cff)
（正式的、随版本更新的引用记录）。GitHub 的 "Cite this repository" 按钮
（位于仓库页面顶部）会直接读取该文件，并支持导出 APA 或 BibTeX 格式。

---

## 安全

请通过 [GitHub 私密漏洞报告](https://github.com/kent-tokyo/renkin/security/advisories/new) 报告安全漏洞。详见 [SECURITY.md](SECURITY.md)。

---

## 许可证

MIT

---

*GitHub Topics: `retrosynthesis` `cheminformatics` `wasm` `rust` `drug-discovery` `casp` `synthesis-planning` `computational-chemistry`*

---

如果 RENKIN 帮您节省了时间，欢迎点个 GitHub star，帮助更多人发现这个项目。
