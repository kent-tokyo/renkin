# RENKIN Roadmap

更新日: **2026-09-26**
基準リリース: **v1.0.9**（`v1.0.9`）

RENKINは、routeを見つけるだけでなく、どのstock・policy・検証根拠で
採用または棄却されたかを再現可能にするCASP基盤である。探索性能、化学的な
健全性、実行資源、監査可能性を分けて評価し、一つの測定値を全体的な優位性へ
拡大解釈しない。

この文書は**現在の優先順位と完了条件**だけを扱う。実装済み変更は
[CHANGELOG.md](CHANGELOG.md)、詳細な作業記録は[tasks/todo.md](tasks/todo.md)、
凍結測定のraw artifactは`data/comparison/`、公開結果は
[benchmark overview](docs/benchmark.md)を正本とする。

## 現在位置

| 証拠 | 確認できること | 主張しないこと |
| --- | --- | --- |
| Phase 55 r2 独立TEST | 固定した690 target、shared stock、asset、budgetでRENKIN 481/690、AiZynthFinder 4.4.1 32/690。paired差 +65.07 pp（95% CI +61.45〜+68.55） | v1.0.9の再測定、普遍的優位性、実験的成功、whole-cohort速度優位性 |
| 旧VAL-200 | 保存済みnative endpointは134/200対134/200 | 新releaseの結果、同時実行された優位性検定 |
| O8 / v1.0.9 | typed route diagnostics、atom-map receipt、CLI/Python/WASM/MCP capability contract | map補完、汎用route repair、MCPによる任意外部route import |

Phase 55のRSSとtime-to-first-routeは`not_measured`のままである。coverageの
結果を、後から速度比較へ読み替えない。

## 優先順位

| 順位 | Track | Status | 次の成果物 | 完了条件 |
| --- | --- | --- | --- | --- |
| P0 | **55.0 Performance receipt** | Active | cold/warm、process-tree RSS、first-route event、timeoutを定義した補足measurement contract | 両armの同一資源条件・入力hash・raw rowを保存し、未計測値を推測で埋めない |
| P0 | **O8.3 Interop + browser evidence** | Planned | SynPlanner 1.7.0のfield別loss reportと、local input artifact→audit→exportのbrowser導線 | preserved/normalized/inferred/dropped/unsupportedをfieldごとに表示し、remote取得や無断修正をしない |
| P1 | **55.x Accuracy diagnosis** | Gated | 失敗targetの最初の損失点に対応する単一仮説のA/B | 同一cohort・stock・budgetで、invalid/regressionなしを確認してから採用 |
| P1 | **O8.4 Bounded repair proposal** | Planned | 1〜2 protecting-group familyだけのopt-in修正提案 | 元routeとbefore/after hash、追加stock、残存riskを保持し、全候補を再監査 |
| P2 | **O7 operational validation** | Continuous | 実procedureとsource artifactの追加検証 | 不足量・条件・単位は`not_evaluable`のまま残す |
| P2 | **Upstream compatibility** | Continuous | AiZynthFinder 4.4.1、Syntheseus 0.8.0、SynPlanner fixtureの回帰 | upstream release/tag/assetを固定し、adapterの対応範囲を過大主張しない |

## Track detail

### 55.0 — performance receipt

まず比較の定義を固定する。対象tool、revision/model、stock、template、cohort、
CPU/RAM/network制約、cold/warmの扱い、timeout、first-routeの時点、RSSの取得方法を
manifestに書く。両armが完了し、検証scriptがrow completenessとinput hashを確認して
初めて比較値を出す。Phase 55 r2のcoverage結果は置換しない。

### O8.3 — interoperability and browser evidence

RENKINの役割は第三のroute schemaを作ることではない。外部routeを受けた際に、
何を保持し、正規化し、推定し、落とし、未対応としたかを明示する。入力artifactは
content hashと変換履歴を持ち、browserではlocal-by-defaultを維持する。

### O8.4 — bounded repair

修正は診断の後段であり、標準探索の暗黙挙動にしない。候補は明示opt-in、限定family、
固定予算で生成し、元routeと併記する。条件妥当性、収率、実験可否を推測しない。

## Release discipline

- 版上げは、ひとまとまりの機能・docs・CHANGELOG・全binding回帰を含む候補commitに対して行う。
- `cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`、format、package、Python、WASM、MCP、docsを変更範囲に応じて検証する。
- benchmark、公開、registry publish、GitHub Releaseの成功は別々に記録する。ローカルtestだけで公開済みとは扱わない。
- 新しいbenchmarkは、測定中のrevision/configurationを凍結する。結果が悪いこと自体を理由に設定を差し替えない。

## 製品境界と非目標

- **chematic**は分子・reactionの低レイヤ、RENKINはroute graph、search、stock policy、audit receiptを担当する。第二のparserやreaction engineを作らない。
- stock identityはstandardize後のcanonical SMILES完全一致であり、substructure matchではない。
- auditの`pass`/`fail`/`partial`は実験的成功、収率、安全性、規制適合を表さない。
- WASMの取消はWorker terminate/respawnであり、coreの協調的cancel APIではない。
- MCPはstdioの公開tool surfaceに限定し、任意の外部route upload/importを暗黙には提供しない。

## Evidence

- [Phase 55 r2 result](docs/benchmark/phase55-r2-result-20260916.md)
- [Metric truth table](docs/benchmark/phase55-metric-truth-table.md)
- [O7 operational validation](docs/benchmark/o7-operational-validation-20260916.md)
- [Trusted route operations](docs/guides/trusted-route-operations.md)
- [MCP capability contract](docs/guides/mcp.md)
