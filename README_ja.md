# RENKIN

Rustで実装した逆合成計画・route監査ツールです。

[English](README.md) · [中文](README_zh.md) · [ドキュメント](https://kent-tokyo.github.io/renkin/) · [Playground](https://kent-tokyo.github.io/renkin/playground/)

RENKINには2つの役割があります。

- **Planner**：目標分子から市販building blockまでの逆合成routeを探索
- **Bridge**：RENKIN、AiZynthFinder、Syntheseus、SynPlannerのrouteを監査

監査はローカルで完結し、構造整合性、stock充足、forward replay、provenance、
再現可能なaudit manifestを確認します。

## インストール

```bash
pip install renkin
cargo add renkin
npm install renkin
```

Syntheseus対応が必要な場合：

```bash
pip install 'renkin[syntheseus]'
```

## Routeを監査する

```python
import json
import renkin

report = json.loads(
    renkin.audit_route(open("trees.json").read(), format="aizynthfinder")
)
print(report["summary"])
```

`format`には`aizynthfinder`、`syntheseus`、`synplanner`、`renkin`を指定できます。
どの入力も同じ監査パイプラインで処理されます。

```bash
renkin audit-route route.json --format auto --output json
```

詳細：[監査ガイド](https://kent-tokyo.github.io/renkin/guides/audit-reproducibility-contract/)、
[private stock policy](https://kent-tokyo.github.io/renkin/guides/private-stock-policy/)、
[route interchange](https://kent-tokyo.github.io/renkin/guides/evidence-carrying-interchange/)

## Routeを探索する

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

CLIでは次のように実行できます。

```bash
cargo run --release -- \
  --target "CC(=O)Oc1ccccc1C(=O)O" \
  --depth 5 --beam-width 100 --format tree
```

エンジンはA*/AND-OR探索、template index、beam制限、stock-aware scoring、
forward validationを備えています。[API](https://docs.rs/renkin) ·
[逆合成ガイド](https://kent-tokyo.github.io/renkin/guides/rust-retrosynthesis/)

## コンポーネント

| Component | 役割 |
| --- | --- |
| `renkin` | planner、CLI、Python binding、WASM |
| `renkin-forward` | forward prediction、enumeration、hint、validation |
| `renkin-kg` | reaction knowledge graph出力 |
| `renkin-mcp` | ローカルMCP server |

化学処理には[`chematic`](https://docs.rs/chematic/)を使用しています。

## MCP

```bash
cargo run --release --bin renkin-mcp
```

stdio経由でroute探索、validation、説明、制約、diagnostics、audit receiptを提供します。
[MCPガイド](https://kent-tokyo.github.io/renkin/guides/mcp/)

## ベンチマーク

最新の正式比較は、明示したshared-stock条件で実施しています。RENKINとAiZynthFinderは
同数のrouteに到達しましたが、普遍的なCASP優位性を証明する結果ではありません。
成功率、速度、stock定義、validation、route品質は分けて報告します。

[ベンチマーク詳細](https://kent-tokyo.github.io/renkin/benchmark/)

## 開発

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

詳細は[`AGENTS.md`](AGENTS.md)、[`tasks/lessons.md`](tasks/lessons.md)、
[`ROADMAP.md`](ROADMAP.md)を参照してください。

## ライセンス

MIT。[LICENSE](LICENSE)
