# RENKIN

Rustで実装した、ローカル実行の逆合成計画・route監査ツールです。

[English](README.md) · [中文](README_zh.md) · [ドキュメント](https://kent-tokyo.github.io/renkin/) · [Playground](https://kent-tokyo.github.io/renkin/playground/)

RENKINには2つの役割があります。

- **Planner**：target SMILESから、指定したbuilding blockまでの逆合成routeを探索します。
- **Bridge**：RENKIN、AiZynthFinder、Syntheseus、SynPlannerのrouteを、同じ決定論的な検査で監査します。

現行リリースは **v1.0.9**。CLI、Rust crate、Python、MCP、ブラウザWASMを提供し、core chemistryにC/C++依存はありません。

## インストール

```bash
pip install renkin
cargo add renkin
npm install renkin
```

## まず使う

aspirinを探索します。

```bash
renkin --target "CC(=O)Oc1ccccc1C(=O)O" --depth 5 --beam-width 100
```

外部routeをローカルで監査します。

```bash
renkin audit-route route.json --format auto --output json
```

Pythonでも同じengineを呼び出せます。

```python
import json, renkin
routes = json.loads(renkin.find_routes("CC(=O)Oc1ccccc1C(=O)O", depth=5))
report = json.loads(renkin.audit_route(open("route.json").read(), format="auto"))
```

## 用途別の入口

| 用途 | 参照先 |
| --- | --- |
| ブラウザだけで探索・監査 | [Playground](https://kent-tokyo.github.io/renkin/playground/) · [WASM API](https://kent-tokyo.github.io/renkin/api/wasm/) |
| アプリケーション連携 | [Python](https://kent-tokyo.github.io/renkin/api/python/) · [Rust](https://docs.rs/renkin) |
| ローカルagent workflow | [MCPガイド](https://kent-tokyo.github.io/renkin/guides/mcp/) |
| private stock・証跡を含む監査 | [監査](https://kent-tokyo.github.io/renkin/guides/audit-reproducibility-contract/) · [policy](https://kent-tokyo.github.io/renkin/guides/private-stock-policy/) |

CLI、Python、WASM、modern MCPは、各surfaceの実効上限をcapability payloadで公開します。private inputは、利用者が明示的にexportしない限りローカルに留まります。

## 範囲とbenchmarkの読み方

監査結果はevidenceであり、実験成功・収率・安全性を保証しません。stock identityはstandardize後のcanonical SMILES完全一致で判定し、外部modelやimportしたrouteを無検証では採用しません。

登録済みPhase 55 shared-stock TESTでは、固定したcohort、stock、asset、budgetの下でRENKINは481/690（69.71%）、AiZynthFinder 4.4.1は32/690（4.64%）のstrict routeでした。この値は普遍的なCASP優位性、実験的妥当性、全cohortでの速度優位性を示すものではありません。[結果record](https://kent-tokyo.github.io/renkin/benchmark/phase55-r2-result-20260916/)を確認してください。

## 開発

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

設計・貢献・現行計画は[AGENTS.md](AGENTS.md)、[CONTRIBUTING.md](CONTRIBUTING.md)、[ROADMAP.md](ROADMAP.md)を参照してください。完全なrelease履歴は[CHANGELOG.md](CHANGELOG.md)に残します。

## ライセンス

MIT。詳しくは[LICENSE](LICENSE)。
