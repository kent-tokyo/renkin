# RENKIN — Retrosynthesis Engine for Knowledge-Informed Navigation

> **コンピュータ支援合成計画（CASP）· Pure Rust · WebAssembly · Python**  
> 錬金（れんきん）― 錬金術のように、目標分子を安価な原料へと逆変換する。

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

[English README](./README.md) · [中文版 README](./README_zh.md) · [**ドキュメント**](https://kent-tokyo.github.io/renkin/) · [**ライブデモ →**](https://kent-tokyo.github.io/renkin/playground/)

---

**Keep your planner. Audit every route.** AiZynthFinder・Syntheseus・RENKIN——どのツールで生成したrouteでも、ローカルで・再現可能な形で・分子構造を一切外部送信せずに監査できます。

[**ブラウザでrouteを監査 →**](https://kent-tokyo.github.io/renkin/playground/) · [**Pythonクイックスタート ↓**](#routeを監査する) · [**route計画エンジン ↓**](#クイックスタート)

---

## RENKINとは

RENKIN Bridgeは、ツール非依存の**route監査ツール**です。**AiZynthFinder**・**Syntheseus**・RENKIN自身のplanner——どのツールが生成したrouteでも、構造整合性・stock充足・宣言済み反応のforward-replay検証を、全く同じ`pass`/`fail`/`partial`パイプラインで判定します。完全にローカルで完結し、監査ごとに検証可能な[`audit_manifest`](https://kent-tokyo.github.io/renkin/guides/audit-reproducibility-contract/)を記録——明示的に指定しない限り、分子構造がお使いのマシンの外へ出ることはありません。

RENKINはそれ自体、目標分子（ゴール）から逆算して市販の安価な原料へと至る化学反応経路を自動発見する、独立した**逆合成（Retrosynthesis）エンジン**でもあります。**創薬・医薬化学・ケモインフォマティクス**において中心的な問題を解きます。

Rust言語と [`chematic`](https://docs.rs/chematic/) クレートで実装された純粋なRust製エンジン——C/C++依存ゼロ、全クレートに `#![forbid(unsafe_code)]` を適用。単一のコードベースがネイティブCLI・Rustライブラリ・Pythonホイール（PyO3）・ブラウザ上で完全にクライアントサイド動作するWebAssemblyモジュールへとコンパイルされます。

---

## インストール

```bash
pip install renkin          # Python
cargo add renkin            # Rust
npm install renkin          # JavaScript (browser / bundler -- see docs/api/wasm.md)
```

Syntheseus routeの監査には、もう一つ任意パッケージが必要です：
`pip install renkin[syntheseus]`（Syntheseus `0.7.2`・`0.8.0`で検証済み——詳細は
互換性はSyntheseus `0.7.2`・`0.8.0`で確認済み）。

---

## ライブプレイグラウンド

**[→ 今すぐ試す](https://kent-tokyo.github.io/renkin/playground/)** — ブラウザ上で完全にWebAssemblyとして動作。インストール不要、サーバー不要、ネットワーク通信なし。

---

## Routeを監査する

すでに手元にあるrouteを、どのツールで計画したものでも持ち込めます——以下のどの経路も全く同じ監査パイプラインを通るため、どのツールが生成したrouteでも、CLI・Python・ブラウザタブのどれで実行しても、同じ`pass`/`fail`/`partial`判定が返ります。

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

**ブラウザで** — インストール不要、アップロード不要、サーバー不要：[**Playgroundを試す →**](https://kent-tokyo.github.io/renkin/playground/)

実物出力による完全なウォークスルー：[AiZynthFinder](https://kent-tokyo.github.io/renkin/guides/aizynthfinder-audit-demo/)（英語）・[Syntheseus](https://kent-tokyo.github.io/renkin/guides/syntheseus-audit-demo/)（英語）。

---

## クイックスタート

*ゼロからrouteを計画する場合はこちら。すでに手元にあるrouteを監査したい場合は上記「[Routeを監査する](#routeを監査する)」を参照。*

```python
import json
import renkin

result = json.loads(
    renkin.find_routes(
        target="CC(=O)Oc1ccccc1C(=O)O",  # アスピリン
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
```

`--format mermaid` で GitHub/Notion 対応フローチャートも出力できます。

[![Open In Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/github/kent-tokyo/renkin/blob/master/examples/renkin_quickstart.ipynb)

---

## 現在の制約

⚠️ ベンチマーク数値はvalidator精度修正後、再計測が進行中です——このリポジトリの他箇所にある78.0%/95.9%/81.8%(ChEMBL)はこの修正より前の値であり、無効化されています。RENKINは収率・実験的に較正された成功確率・副反応を予測せず、文献の自動検索も行いません（`success_probability`はtemplate頻度由来の探索スコアであり、較正された予測値ではありません）。修正済みの過去計測値・詳細な手法・既知の制約は[ベンチマーク](https://kent-tokyo.github.io/renkin/benchmark/)を参照してください（このページは単一commit時点で凍結された過去の計測であり、リアルタイムの数値ではありません）。

---

## RENKIN を選ぶ理由

RENKIN は Rust ネイティブの合成計画スタックとして設計されています：

| | |
|---|---|
| **高速** | A\* / AND-OR ツリー探索 · ビームサーチ · テンプレート頻度重み付け |
| **ポータブル** | CLI · Python · npm/WASM · ブラウザ Playground をひとつのコードベースで |
| **説明可能** | ステップごとに `confidence`・`atom_economy`・`route_cost`・`procedure_hint` |
| **検証可能** | `renkin-forward` が各逆合成ステップをフォワード適用で検証 |
| **ベンチマーク対応** | USPTO-50k・PaRoutes 形式評価・ルート多様性・原子収支チェック |
| **AIエージェント対応** | MCP サーバーで Claude Desktop 等への経路・検証ツール公開 |

---

## 制約付き探索

出発原料の元素組成で探索を制限できます。

**デフォルト探索** — ビフェニルの5ルート:

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

**制約付き探索** — ボロン酸カップリングのみ（Br・I 出発原料を除外）:

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

制約は自由に組み合わせ可能。探索後フィルタとして適用されるため、A\* 探索自体は変化しません。

---

## テンプレート evidence メタデータ

extracted templateはファイル内の位置に基づく表示名（`extracted_{i}`）しか持たず、
ファイルの並び替えや再抽出のたびに変わってしまうため、DOI・報告収率・既知の副反応
といった外部知識を永続的に紐づけられませんでした。すべてのテンプレート（hand-crafted
とextracted両方）に、代わりに安定した `template_id` が付与されます:

- hand-crafted rule: `rule:<rule_name>`（例: `rule:suzuki_retro`）
- extracted template: `smirks-sha256:<hex>` — trim済みSMIRKS文字列のSHA-256 hex
  digest。ファイル内の位置・読み込み順・count値に依存しない。純粋に構文的な値であり
  SMIRKSの意味的canonicalizationは行わない（同じ意味でも表記が異なるSMIRKSは別IDになる）。

`renkin template ids <file.smi>` を実行すると、各テンプレートの `template_id`・
表示名・SMIRKS・weightの一覧を出力する（既定はTSV、`--format json` でJSON出力）。
サイドカーファイル作成時のID確認に使用する。

`--template-metadata sidecar.json`（Pythonでは
`find_routes(..., template_metadata_path=...)`）で、`template_id` をキーとする
evidenceを紐づけられる:

```json
{
  "schema_version": 1,
  "templates": {
    "smirks-sha256:ef8778a2888469d619c52cce7e74f6848e101049050dd1b765b78f32e3c94498": {
      "references": [
        { "id": "ref-1", "kind": "doi", "identifier": "10.xxxx/example" }
      ],
      "condition_candidates": [
        {
          "catalysts": ["Pd(PPh3)4"],
          "bases": ["K2CO3"],
          "solvents": ["EtOH", "water"],
          "temperature_c": { "min": 75.0, "max": 85.0 },
          "source": "literature",
          "scope": "template",
          "reference_ids": ["ref-1"]
        }
      ],
      "reported_yields": [
        {
          "percentage": { "min": 72.0, "max": 81.0 },
          "basis": "isolated",
          "source": "literature",
          "scope": "template",
          "reference_ids": ["ref-1"]
        }
      ],
      "warnings": [
        {
          "code": "possible_protodeboronation",
          "severity": "medium",
          "message": "Protodeboronation has been reported under prolonged aqueous heating.",
          "source": "literature",
          "scope": "template",
          "reference_ids": ["ref-1"]
        }
      ]
    }
  }
}
```

一致したステップには `condition_candidates`・`reported_yields`・`references`・
`warnings` を含む `evidence` フィールドが付与され、サイドカーに該当エントリがない
テンプレートを使うステップには `evidence` キー自体が存在しません。サイドカーは
**探索が始まる前に**読み込み・検証されます（schema_version・重複/存在しない
reference ID・収率の範囲・range の `min <= max`・DOI/特許識別子の非空チェック）。
不正なメタデータはハードエラーになり、ロードされたルールに一致しない `template_id`
がサイドカーにある場合は警告のみ（失敗はしない）です。

**これは何ではないか:**
- `reported_yields` は外部で報告された値を記録したものであり、**RENKIN自身の予測
  ではありません**。`step_confidence`/`success_probability` はこれに影響されず、
  引き続きtemplate frequency由来の探索スコアであって実験成功率ではありません。
- `warnings` は与えたサイドカーに明示的に含まれる内容のみを反映し、**自動的な
  副反応検出ではありません**。
- サイドカーに該当エントリがないテンプレートには、evidenceは一切捏造されません。

収率・成功率の自動予測や文献の自動検索は本フェーズのスコープ外であり、
[#41](https://github.com/kent-tokyo/renkin/issues/41) で今後のタスクとして
追跡されています。

### 基質固有のexample（`schema_version: 2`）

ここまでの内容はすべて**テンプレート単位**の evidence であり、対象分子に関わらず
そのテンプレートを使う全ステップに適用されます。`schema_version: 2` では、
`examples` という配列が追加され、各エントリは「この対象・この原料の組み合わせで
実際に報告された1件の記録」を表します（`target_smiles`/`precursor_smiles` で
キーづけ）:

```json
{
  "schema_version": 2,
  "templates": {
    "smirks-sha256:...": {
      "references": [{ "id": "ref-1", "kind": "doi", "identifier": "10.xxxx/example" }],
      "examples": [{
        "id": "ex-1",
        "target_smiles": "c1ccc(-c2ccccc2)cc1",
        "precursor_smiles": ["Brc1ccccc1", "c1ccccc1"],
        "conditions": { "catalysts": ["Pd(PPh3)4"], "solvents": ["EtOH"], "source": "literature", "scope": "substrate_specific", "reference_ids": ["ref-1"] },
        "reported_yield": { "percentage": 78.0, "basis": "isolated", "source": "literature", "scope": "substrate_specific", "reference_ids": ["ref-1"] },
        "reference_ids": ["ref-1"]
      }]
    }
  }
}
```

`examples` は `schema_version: 2` でのみ使用可能です（`1` で指定するとハード
エラーになります）。`schema_version: 2` では報告収率も `examples[].reported_yield`
に置く必要があり、テンプレート単位の `reported_yields` が非空だとハード
エラーになります（`schema_version: 1` では引き続き許可されます）——
これにより、基質固有の数値がそのテンプレートを使う全ステップへ漏れ伝わる
ことを防ぎます。example内の condition/yield/warning はすべて
`substrate_specific` スコープでなければなりません。

ルートのステップの `evidence.examples` は、サイドカーからの単純コピーでは
なく**解決済み**の状態で渡されます：canonical化したtarget SMILESと、
canonical化・順序非依存でsort＋dedupしたprecursor集合の両方でそのステップと
照合され（サイドカー内の `precursor_smiles` の順序を変えても結果は
変わりません）、exact substrate matchはすべて保持し、同一テンプレート・
異なる基質のexampleは最大3件までに制限されます。各エントリはJSON上に
`match_kind`（`exact_substrate`/`template_only`）を持ち、
`template_examples_total` で宣言されていたexample総数も分かるため、
`--format explain` だけでなくJSON／Pythonの利用者も「この反応そのものの
evidence」と「別基質の文献上の前例」を機械的に区別できます。
`--format explain` ではexact substrate matchを先に表示し、それぞれ
「Exact substrate example:」または「different substrate; not a
prediction」と明記されます。`conditions`／`reported_yield`／`warnings`は
それぞれ自身が引用するreferenceを直下に表示し（同じreferenceが複数箇所で
使われても重複表示はしません）。詳細なマッチング／検証仕様は
[Reaction Evidence guide](docs/guides/reaction-evidence.md#substrate-specific-examples-schema_version-2)
を参照してください。

**ORDからのevidenceインポート。** `renkin evidence match`(exact-setの
バッチtemplate matching、fuzzy/類似度matchingなし)と
[`scripts/ord_evidence_audit.py`](scripts/README_ord_evidence.md)(オフライン、
ネットワークアクセスなし)により、ローカルにダウンロード済みの
[Open Reaction Database](https://github.com/open-reaction-database/ord-data)
corpusを`schema_version: 2`のsidecarへ変換できます。採用されたrecordはRENKIN
自身のloaderで再検証され、一意にmatchせず・provenanceが確認できないものは
推測せずaudit reportへ除外理由付きで記録されます。RENKIN自体が文献検索を行う
ことはなく、reported yieldは予測値ではありません。ORDのreaction dataは
CC-BY-SA-4.0であり、RENKIN本体コードのMITとは別ライセンスです。詳細な採用
基準とライセンスの分離については
[Reaction Evidence guide](docs/guides/reaction-evidence.md#importing-from-ord-open-reaction-database)
を参照してください。

---

## 特徴

| 特徴 | 詳細 |
|---|---|
| **Pure Safe Rust** | 全クレートに `#![forbid(unsafe_code)]` — コンパイラ保証、C/C++依存ゼロ |
| **探索エンジン** | A\*/AND-OR木探索（Retro\*相当、プラガブルな`MoleculeValueEstimator`/`ReactionPrior`）、メモリ制約付き探索の`--beam-width N`、非WASM環境での`rayon`並列ルール適用（wasm32はシーケンシャルフォールバック） |
| **最大50k逆合成テンプレート** | USPTO-50k/MIT からrdchiralで自動抽出；頻度重み付け優先（オプションのpure-Rust `tract-onnx` NNスコアラーを`--scorer`で利用可）；`--templates` でカスタムセット対応 |
| **テンプレート品質ツール** | `renkin template stats\|validate\|dedup\|explain\|coverage\|ids` — 頻度分布・有効性・重複・検索・カバレッジ・安定IDを検査 |
| **安定 template_id + evidence サイドカー** | すべてのテンプレートに安定した `template_id` を付与——hand-crafted ruleは `rule:<name>`、extracted templateは `smirks-sha256:<hex>`（ファイル内の並び順に依存しない）。`--template-metadata sidecar.json` でDOI・特許・報告済み条件・報告済み収率・既知の副反応警告を紐づけ可能。一致したステップにのみ `evidence` フィールドが付与される——詳細は下記「[テンプレート evidence メタデータ](#テンプレート-evidence-メタデータ)」参照。`schema_version: 2` サイドカーではさらに `examples`（exact substrate match、`--format explain` で優先表示）を紐づけ可能。収率・成功率の自動予測や文献自動検索は引き続きスコープ外（[#41](https://github.com/kent-tokyo/renkin/issues/41)） |
| **Ring-context安全ガード** | `--ring-context-policy conservative --ring-context-sidecar <path>` — extracted templateの環開閉切断が、訓練データで一度も環結合として観測されていない場合に拒否するopt-inのmatch-levelフィルタ。デフォルトは `disabled`（既存挙動のまま） — [Issue #72](https://github.com/kent-tokyo/renkin/issues/72)参照 |
| **LightGBM candidate reranker** | `--reranker-model`/`--reranker-freq-table`（CLI）または `reranker_model_path`/`reranker_freq_table_path`（Python）— opt-inかつordering-onlyな再順位付け。生成される候補そのものは一切変更されず探索順序のみが変わり、オフ時はレガシー順序とbyte単位で完全一致。Paired 100-target route-searchゲート: `route_to_configured_stock` 16→20（+4/-0）。`python3 scripts/fetch_reranker_model.py` で凍結モデルを取得（SHA-256検証つき、パッケージには同梱しない — [ロードマップ](#ロードマップ)参照） |
| **Coverage mode（opt-in）** | `--search-mode coverage --coverage-templates <path>`（CLI）または `search_mode="coverage"`, `coverage_templates_path=...`（Python）— デフォルトのテンプレートセットでルートが見つからない場合のみ、自動的により大きな別テンプレートセットへエスカレーションする。`--coverage-timeout-secs` で協調的にキャンセル可能。未使用時の標準モード出力はbyte単位で完全不変。`python3 scripts/fetch_coverage_templates.py` で凍結済み2,000テンプレートのStage-2セットを取得（SHA-256検証つき、パッケージには同梱しない、rerankerモデルと同じ理由 — [ロードマップ](#ロードマップ)参照） |
| **RENKIN Bridge / `audit-route`** | `renkin audit-route route.json [--format auto\|renkin\|aizynthfinder\|syntheseus] [--stock stock.smi] [--output human\|json]` — ツール非依存のroute audit: 構造整合性・stock・宣言済み反応のforward replay検証を、それぞれ独立に `pass`/`fail`/`not_evaluable` で報告し、route全体は `pass`/`fail`/`partial` で判定。RENKIN-native route JSON（v0.25.0）、実物AiZynthFinder route JSON（単一ターゲット・gzip圧縮batch出力、AiZynthFinder 4.3.2／4.4.0／4.4.1で検証済み——全バージョン対応を主張するものではない、v0.26.0、v0.32.0でversion matrixを拡大）、そしてSyntheseus route（Syntheseus自体にはネイティブのroute export機能がないため、任意インストールの`renkin.syntheseus_exporter`が生成する`syntheseus-route-v1`交換schema経由、v0.30.0）に対応。`--format auto` は入力の形状から判定し、曖昧な場合は推測せずエラーにする。[AiZynthFinderデモ →](https://kent-tokyo.github.io/renkin/guides/aizynthfinder-audit-demo/)（英語）・[Syntheseusデモ →](https://kent-tokyo.github.io/renkin/guides/syntheseus-audit-demo/)（英語） |
| **ルートスコアリング** | `confidence`, `step_confidence`, `success_probability`（Retro-prob方式）, `convergency`, `atom_economy`, `route_cost`（`Σ(BB価格) + ステップ数×0.5`、または`--bb-prices`/`--stock`で実価格）— 下の注記も参照 |
| **ステップメタデータの出所表示** | 各ステップに `metadata_source`/`metadata_scope` を付与し、`conditions`/`reaction_family` がルール作者による既定値かそれ以上の根拠があるかを機械可読に区別。extracted templateには付与しない（捏造しない） |
| **Pareto多目的探索** | `--format pareto` で `route_cost`・`success_probability`・`steps` 等のパレートフロントを返す；`--objectives` で目的関数をカスタム設定 |
| **制約 DSL** | `--constraints constraints.json` — 元素フィルタ・ステップ/コスト制限・信頼度閾値・必須/除外/優先反応族；LLM → RENKIN パイプラインに対応 |
| **出力フォーマット & 診断** | `--format json\|tree\|mermaid\|explain\|compare\|compare-json\|pareto`；ルートが見つからない場合はJSONに `diagnostics`（`likely_causes`/`suggestions`）を付加 |
| **`renkin-forward` ツール群** | `predict`（順反応生成物のランキング）、`enumerate`（1反応物+partnerライブラリからの境界付き列挙）、`hints`（partner不要の検索用ヒント、具体的生成物は出さない）、`validate`（各retroステップの順方向検証）— [Forward guides](docs/guides/forward-retrieval-hints.md#predict--enumerate--hints-at-a-glance)（英語）参照 |
| **`renkin-bench`** | USPTO-50k/PaRoutes評価、`--plausibility`（順方向検証済み複合スコア）、`--failure-taxonomy`、原子収支チェック（`target_MW > Σ precursor_MW`）、未解決ターゲットを対象にした多段階`cascade`再実行 — [ベンチマーク](#ベンチマーク)参照 |
| **stock CSV 管理** | `renkin stock stats\|validate\|coverage` — SMILES・名称・ベンダー・価格・ハザード情報 |
| **MCPサーバー** | `renkin-mcp` が 6 ツールを提供：`find_routes`, `validate_route`, `explain_route`, `find_pareto_routes`, `plan_with_constraints`, `estimate_diversity` |
| **`renkin-doctor`** | 環境診断バイナリ — テンプレート・市販品データ・Python インポート・ツールバージョン・データ整合性を検査 |
| **`renkin-kg`** | 反応知識グラフ構築ツール — ルートから分子↔反応の二部グラフを生成；GraphML / Cypher 形式でエクスポート |
| **マルチターゲット** | `pip install renkin`（Linux/macOS/Windows プリビルドwheels）· `npm install renkin`（~500 KB WASM、ブラウザでネイティブに近い速度） |
| **市販原料 + 立体化学** | `data/building_blocks.smi` から実際にロードされたユニーク化合物402件（アリールハライド、ボロン酸、ヘテロ環、アミン、酸、アミノ酸——[ベンチマーク](#ベンチマーク)参照）；完全な四面体@/@@およびE/Z立体化学サポート；各ルートJSONに`building_blocks`フィールド（末端原料のSMILES、手動パース不要） |

> **`step_confidence`/`success_probability` は収率でも実測の成功率でもない。**
> テンプレート出現頻度から導かれる探索順位付けスコア（`rule_weight / max_rule_weight` をステップ間で乗算）であり、
> 実験的成功確率のキャリブレーション値でも、期待単離収率でもない。ルート単位の実験的収率・成功率報告は未実装。

---

## ベンチマーク

USPTO-50kテストセット（全4,907分子評価）:

> **評価定義**: `find_routes` がbuilding block集合に含まれる末端precursorのみで構成される経路を depth=5・beam=100 以内で1件以上見つけられれば solved。USPTO-50k の正解試薬とは照合しない。

### Corrected baseline（コミット `e20dc8c`、2026-07-22）

| Public label | 指標 | 値 |
|---|---|---|
| Search-to-stock rate | `raw_solved_rate` | **20.09%**（986/4,907） |
| Atom-balance-filtered rate | `atom_balanced_solved_rate` | **15.41%**（756/4,907）— search-to-stockの部分集合 |
| Current-validator-confirmed rate | `provenance_validated_solved_rate` | **0.88%**（43/4,907）— atom-balance-filteredの部分集合 |

402件（`data/building_blocks.smi`から実際にロードされたユニーク化合物数）の市販ビルディングブロック、5,000件の抽出テンプレート、28件のハンドクラフトルール、depth=5・beam=100。3つの数値は同一4,907件に対する入れ子系列であり、独立した数値として比較しないこと。いずれも実験的に検証された合成成功率や人間の化学者によるルート正確性評価ではない。`provenance_validated_solved_rate`は実測の化学的正確性の値でも、証明された正確性の下限でもない——現行validatorが確認できたルートのみを数えており、「invalid」判定のうち未知の割合がvalidator偽陰性である可能性がある一方、実際のrule・route誤りも含まれ得る（比率未計測）。詳細な手法・rule別内訳・再現コマンドは [`tasks/phase31_final_remeasurement_run.md`](https://github.com/kent-tokyo/renkin/blob/master/tasks/phase31_final_remeasurement_run.md) · [ベンチマーク詳細 →](https://kent-tokyo.github.io/renkin/benchmark/) を参照してください。

### 過去の推移（修正前・無効化済み — 冒頭の注記参照）

⚠️ 以下の数値（単一パス78.0%、cascade 95.9%、ChEMBL OOD 81.8%）は31.11/31.12修正前の計測であり無効化済み、再計測もされていません。参考として残していますが、現在のRENKINの性能として引用しないでください。

> **評価条件の注記**: 全数値は USPTO-50k の標準 train/test 分割（同一コーパス）を使用。テンプレートは訓練セットから抽出しテストセットで評価——AiZynthFinder 等の論文と同じ手法。数値は USPTO-50k ドメイン内での性能を示すものであり、分布外（OOD）汎化性は別途検証が必要。

| 設定 | 解決数 | 解決率 | BB数 | テンプレート数 | depth | beam | ms/mol |
|---|---|---|---|---|---|---|---|
| v0.1.0 初期 | 366/4907 | 7.5% | 463 | 31 | 3 | 50 | — |
| 自動テンプレート追加（top-300） | 1363/4907 | 27.8% | 463 | 222 | 3 | 50 | — |
| depth=5 + top-500 テンプレート | 2315/4907 | 47.2% | 463 | 314 | 5 | 50 | — |
| + beam=100 | 2688/4907 | 54.8%* | 463 | 314 | 5 | 100 | — |
| + Phase A（頻度重み付け） | 3540/4907 | 72.1%† | 463 | 314 | 5 | 100 | — |
| + 5,000テンプレート、480 BB | 3826/4907 | 78.0% | 480 | 5,000 | 5 | 100 | 2,775 |
| Phase A 無制限（beam=0） | 3832/4907 | 78.1% | 480 | 5,000 | 5 | 0 | — |
| Phase B（NNスコアラー、tract-onnx） | 3826/4907 | 78.0% | 480 | 5,000 | 5 | 100 | 3,394 |
| **+ diaryl sulfone ルール、509 BB** | **3826/4907** | **78.0%** | **509** | **5,000** | **5** | **100** | **≈2,800** |
| Cascade（stage2: 未解決のみ depth=7, beam=300） | 4705/4907 | **95.9%** | 509 | 5,000 | 7 | 300 | — |

\* 29/50チャンク、旧バイナリ  
† 全50チャンク完了 — **72.1%**（3,540/4,907）確認済  
この過去の表のBB数（463/480/509）は当時の記載値そのままであり、`ChemEnv::bb_count()`による再検証はしていないlegacy documentation value。現在の`data/building_blocks.smi`の実ロード数は上記corrected baselineの402件。

*注意: LocalRetro（53.4%）・GLG（58.0%）は単ステップ top-1 予測精度であり、多段階経路探索成功率とは別の指標のため直接比較不可。*

> **ベンチマーク範囲に関する注意**: USPTO-50k はここでは *標準化されたサニティベンチマーク* として使用しており、実世界の広範な合成性能を証明するものではありません。同コーパスは主に製薬合成で一般的な C–C・C–N 結合形成に偏っており、USPTO の掲載が少ない反応タイプは体系的に不利になります。ChEMBL 承認薬（OOD）での **81.8%**（409/500、修正前・未再計測）はルールセットがテストコーパスを超えて汎化することを示唆していましたが、いずれの過去数値も任意のターゲットに対する経路品質を保証するものではありません。

---

## 競合比較

⚠️ 以下のRENKIN行は修正版の `raw_solved_rate`（20.09%）を使用——過去バージョンのこの表にあった cascade 95.9% は無効化済み・未再計測のためここには含めない。

| ツール | 言語 | ライセンス | WASM | ゼロ依存 | アルゴリズム | テンプレート | 在庫 |
|---|---|---|---|---|---|---|---|
| **ASKCOS** | Python | CC BY-NC | No | No（Docker/64GB） | MCTS + A\* | USPTO（ML） | ZINC |
| **AiZynthFinder** | Python | MIT | No | No（conda+モデル） | MCTS | USPTO（ML/~50k） | eMolecules（~6M） |
| **SYNTHIA** | クローズド | 独自 | No | No | SMARTS+AND/OR | 手動作成 | Sigma-Aldrich |
| **IBM RXN** | クローズド | SaaS | No | No | Transformer | USPTO | — |
| **Retro\*** | Python | MIT | No | No（未メンテ） | A\*+AND/OR | USPTO（ML） | eMolecules |
| **★ RENKIN** | **Rust** | **MIT** | **Yes** | **Yes** | **A\*+AND/OR** | 手動+rdchiral（5k デフォルト；`--templates` で50k対応） | 402+（拡張可） |

`raw_solved_rate`は他plannerが公開しているroute-finding成功率に最も近いRENKIN側の指標だが、stock規模・template集合・target集合・探索budget・route品質検査がシステムごとに異なるため直接比較はできず、この表はRENKINが他システムより優れている（あるいは劣っている）ことを示すものではない。

**RENKINの目標**: GPU なし・学習データなし・ブラックボックスなし——キュレーション済みルールと自動抽出テンプレートだけで、ニューラルネットベースのツールに匹敵する精度を目指す。RENKIN のベンチマーク設定（corrected baseline、コミット `e20dc8c`、2026-07-22）では単一パス `raw_solved_rate` **20.09%**（986/4,907）を達成——入れ子系列のフルセットと、より厳格な `provenance_validated_solved_rate`（0.88%）が実測の正確性の値でも保証された下限でもない理由は上記ベンチマークセクション参照。ブラウザ・CLI・Python、どこでも動く。

---

## MCP サーバー

`renkin-mcp` は逆合成を MCP ツールとして公開し、AI エージェント（Claude 等）から直接呼び出せます。

**設定** — `claude_desktop_config.json` に追加：

```json
{
  "mcpServers": {
    "renkin": { "command": "/path/to/renkin-mcp" }
  }
}
```

**ツール一覧** (6):

| ツール | 説明 |
|---|---|
| `find_routes` | 逆合成：SMILES → スコア付きルート |
| `validate_route` | 逆合成ルートを順方向検証 |
| `explain_route` | ルートごとの強み/弱みを人間可読形式で出力 |
| `find_pareto_routes` | 多目的パレートフロント探索 |
| `plan_with_constraints` | 制約 DSL による合成計画（元素フィルタ・ステップ/コスト・信頼度閾値・必須/除外/優先反応族） |
| `estimate_diversity` | ルート多様性・カバレッジ指標 |

`find_routes` は、必須の `coverage_templates` パスと
`search_mode: "coverage"` を指定すると、coverage modeにも対応します。
Stage 1で見つからない場合だけStage 2へ進み、選択Stage・timeout状態・各Stageの
経過時間を応答に含めます。

```bash
cargo build --release
# binary: target/release/renkin-mcp
```

---

## アーキテクチャ

### ワークスペース全体像

```
┌──────────────────────────────────────────────────────────────────┐
│ renkin workspace（本リポジトリ）                                  │
│                                                                  │
│  renkin（逆合成）                 renkin-forward                   │
│  ──────────────────────           ─────────────────────────────  │
│  target → precursors              reactants → products           │
│  A* / AND-OR 木探索               テンプレートベース順反応予測    │
│  ルートスコアリング・制約         （逆合成ルートの検証に利用）    │
│        │                                    │                    │
│        └──────────────────┬─────────────────┘                    │
│                           ▼                                      │
│               chematic（分子表現・SMILES・部分構造マッチ・        │
│               反応 SMARTS）                                      │
└──────────────────────────────────────────────────────────────────┘
```

### 内部データフロー（renkin クレート）

```
目標 SMILES
     │
     ▼
┌─────────────────────────┐
│     chem_env.rs         │  ← chematic ラッパー
│  - SMILES パース        │     canonical-SMILES FxHashSet BB照合（O(1)）
│  - 5,000 逆反応ルール   │     フラグメント正規化・リークフィルタ
│  - 市販品チェック       │     apply_retro メモ化キャッシュ
└────────────┬────────────┘
             │  par_iter (rayon / WASM では逐次)
             ▼
┌─────────────────────────┐
│      search.rs          │  ← A* / AND-OR 木探索
│  - 優先度付きキュー     │     SA Score ヒューリスティック + メモ化
│  - クローズドリスト     │     ビームサーチ（SmallVec フロンティア）
│  - Arc<PathNode> パス   │     子ノードあたり O(1) パス共有
└────────────┬────────────┘
             │
             ▼
┌─────────────────────────┐
│      score.rs           │  ← 評価関数
│  - SA Score             │     h = Σ(1 + 0.5·(sa−1)/9)
│  - 分子量コスト         │     g = Σ(1 + total_mw/2000)
└────────────┬────────────┘
             │
             ▼
┌─────────────────────────┐   （オプション）
│      scorer.rs          │  ← Phase B: NNテンプレートスコアラー
│  - tract-onnx           │     Pure Rust ONNXインファレンス
│  - --scorer フラグ      │     分子固有テンプレートランキング
└────────────┬────────────┘
             │
             ▼
  JSON  ←  CLI / Python / WASM
```

---

## プロジェクト構成

```
renkin/                          ← Cargo workspace ルート
├── Cargo.toml
├── src/                         ← renkin クレート（逆合成）
│   ├── lib.rs                   # ライブラリクレート
│   ├── main.rs                  # CLI バイナリ（--templates, --template-metadata, --scorer, --constraints, --objectives フラグ対応）
│   ├── bin/benchmark.rs         # renkin-bench バイナリ（--plausibility フラグ対応）
│   ├── bin/doctor.rs            # renkin-doctor 環境診断バイナリ
│   ├── bin/fp.rs                # renkin-fp ECFP4 フィンガープリント（nn-scoring フィーチャー）
│   ├── bin/mcp.rs               # renkin-mcp MCP サーバー（6 ツール）
│   ├── chem_env.rs              # 逆合成ルール・市販品判定・テンプレートローダー
│   ├── score.rs                 # SA Score ヒューリスティック
│   ├── search.rs                # A* / AND-OR 木探索エンジン
│   ├── scorer.rs                # Phase B: tract-onnx NNテンプレートスコアラー
│   ├── candidate.rs             # 1ステップ候補提案（オフラインリランキング基盤、探索へは未統合）
│   ├── pool_export.rs           # 候補プール JSONL + 再現性マニフェストのエクスポート
│   ├── python.rs                # PyO3 バインディング
│   └── wasm.rs                  # wasm-bindgen バインディング
├── crates/                      ← 兄弟クレート
│   ├── renkin-forward/          # 順反応予測（reactants → products）
│   └── renkin-kg/               # 反応知識グラフ（分子↔反応 二部グラフ、GraphML/Cypher エクスポート）
├── data/
│   ├── building_blocks.smi              # 402件の市販原料（実ロード・重複除去後の数）
│   ├── templates_extracted_5000.smi     # 5,000件の自動抽出SMIRKSテンプレート
│   ├── benchmark_targets.smi            # 内部ベンチマークセット
│   └── bench_chunks/                    # USPTO-50k チャンク別結果
├── scripts/
│   ├── extract_templates.py         # rdchiral テンプレート抽出パイプライン
│   ├── run_benchmark_chunks.sh      # 再開可能チャンクベンチマーク
│   ├── train_reranker.py            # 候補リランカー訓練/評価（開発ツール、オフライン専用）
│   └── tests/                       # train_reranker.py の unittest スイート
├── docs/                # MkDocs ソース → kent-tokyo.github.io/renkin/
└── mkdocs.yml
```

---

## ロードマップ

全リリース履歴（時系列）: [`CHANGELOG.md`](CHANGELOG.md)（英語）。
このセクションでは現在の主要項目と次の予定のみを扱う——それ以前の出荷済み機能は下記「過去のマイルストーン」参照。

### 最近出荷

- [x] **Syntheseus Bridge**（`--format syntheseus`、v0.30.0で出荷）— *Syntheseusにはroute exportがない。だからRENKINが自前で作った——そして他のどのadapterとも全く同じ方法で監査する。* RENKIN-native・AiZynthFinderに次ぐ3つ目のroute adapter: 任意インストールの`renkin.syntheseus_exporter`（`pip install renkin[syntheseus]`）が、実物のSyntheseus `SynthesisGraph`を`syntheseus-route-v1`交換schemaへ変換し、`renkin audit-route --format syntheseus`（自動判定にも対応）がそれを、他のどのadapterとも全く同じ監査パイプラインで消費する。forward validationは、実物のSyntheseus routeでは今のところ常に`not_evaluable`と正直に報告する——`reaction_smiles`にatom mappingが一切含まれないため、pass判定を偽装することはない。[ブラウザPlayground](https://kent-tokyo.github.io/renkin/playground/)のAudit タブにも、Syntheseusが3つ目のformatオプションとして追加された。[実物出力による5分デモ →](https://kent-tokyo.github.io/renkin/guides/syntheseus-audit-demo/)（英語）
- [x] **Audit Policy Profiles**（`--policy informational|standard|strict`、v0.29.0で出荷）— *findingは1組のまま、判定の導き方だけを3通りに。* 同じrouteを`informational`／`standard`／`strict`のいずれかのpolicyで監査でき、基となるfindingを隠したり変えたりすることは一切ない——policyが変えるのは、既に収集されたfindingから全体のpass/fail/partial判定をどう導出するかだけであり、`audit_manifest.policy`に実際に使われたpolicyが記録される。CLI（`renkin audit-route --policy`）・Rust API・Python初のroute audit binding（`renkin.audit_route()`）・新しいWASM `audit_route_v2()`（既存の`audit_route()`は`standard`policyのwrapperとして維持）のすべてで一貫した挙動。[ブラウザPlayground](https://kent-tokyo.github.io/renkin/playground/)のAudit タブにもpolicy selectorが追加された。
- [x] **Audit Playground**（`[ Audit a Route ]`タブ、v0.28.0で出荷）— *ブラウザ内でrouteを監査——同じパイプライン、同じ判定、通信は一切なし。* [ブラウザPlayground](https://kent-tokyo.github.io/renkin/playground/)が、RENKINまたはAiZynthFinderのroute export（単一route・Pandas batch双方）と任意のstockリストを、ブラウザ内で完結して監査できるようになった。新しい`audit_route` WASM exportは`renkin audit-route`と全く同じレポート生成パイプラインを呼び出すため、どちらで監査してもpass/fail/partial判定は同一——別々に保守される複製ではない。貼り付けまたはアップロードし、メインスレッドをブロックせず実行し、JSONレポートをダウンロードできる。
- [x] **Reproducible Route Audit**（`renkin audit-route --output json`の`audit_manifest`、v0.27.0で出荷）— *何を、どの入力から、どのstock・policyで監査したかを再現できるように。* 監査レポートに、RENKINバージョン・report schema version・source format/version・入力とstockのSHA-256ハッシュ・audit policyを記録するようになった——同一入力を2回監査してbyte-identicalになることをテストで確認済み、単なる主張ではない。RENKIN-nativeとAiZynthFinderの両route入力に共通のadapter conformance suiteを追加し、[再現性・互換性契約のドキュメント](https://kent-tokyo.github.io/renkin/guides/audit-reproducibility-contract/)（verified対supportedバージョンの区別、未知フィールドの許容、report schemaルール、adapter fixture追加手順）も整備した。今回のリリースでは[ブラウザPlayground](https://kent-tokyo.github.io/renkin/playground/)にも安全性・UX面の改善が入っている: 探索処理はメインスレッドをブロックせずキャンセル・タイムバジェットに対応、構造式描画は既定でブラウザ内完結（第三者へのSMILES送信なし）、検索条件はCopy CLI/Pythonで正確に再現できる。
- [x] **RENKIN Bridge — Cross-Tool Route Audit**（`renkin audit-route`、RENKIN-nativeアダプタはv0.25.0で出荷、AiZynthFinderアダプタはv0.26.0で出荷）— *Keep AiZynthFinder. Audit its routes with RENKIN.* ツール非依存のroute audit model: 構造整合性・stock・宣言済み反応のforward replay検証を、それぞれ独立に `pass`/`fail`/`not_evaluable` で報告し、route全体は `pass`/`fail`/`partial` で判定——boolean へ暗黙に握り潰さない。v0.26.0では実物AiZynthFinderアダプタを追加（単一ターゲット・gzip batch JSON、実際にキャプチャしたv4.4.1出力で検証済み——詳細は[`PROVENANCE.md`](tests/fixtures/aizynthfinder/v4.4.1/PROVENANCE.md)参照）、加えて`--format auto`検出により、両ツールのrouteが全く同じ監査パイプラインを通る。実物fixtureでの監査により、前駆体の並び順だけで検証結果が変わってしまう共有のforward-replayバグも発見・修正した。`renkin audit-route route.json --stock stock.smi --output json` で、どちらのツールが生成したrouteでも、ファイル内の全routeを監査しmachine-readableな1つのreportへ集約する
- [x] リランカーを実際に使える形にする: Python面（`find_routes()`の`reranker_model_path`/`reranker_freq_table_path`）とbatteries-includedなモデル配布（`scripts/fetch_reranker_model.py`、v0.22.0 GitHub Releaseの正規assetからSHA-256検証つきで取得）（[#101](https://github.com/kent-tokyo/renkin/issues/101)、v0.23.0で出荷）— v0.22.0でリランカーが効くことを実証済み、v0.23.0はusability/配布面の解禁であり新たな精度向上の主張ではない
- [x] LightGBM候補リランカーをオフライン学習・ゲート通過させ、route searchへ統合（[#101](https://github.com/kent-tokyo/renkin/issues/101) Task 35、CLIはv0.22.0で出荷）— 実USPTO-50kラベルでLambdaMARTモデルを学習、VAL screening gate通過（top1 +11.7pp・MRR +11.3pp・top10 +9.3pp、bootstrap CI確認済み）、frozen modelに対しformal 4,903-target TEST評価を一度だけ実施しPASS（top1 +12.7pp・MRR +11.9pp・top10 +9.1pp — VALと同程度の改善幅でoverfitting兆候なし）、その後`find_routes`へordering-onlyのrank bonusとして統合し、paired 100-target route-searchゲートで確認: `route_to_configured_stock` 16→20/100（+4/-0）。詳細は上記の特徴表参照
- [x] 500-target規模のRENKIN vs AiZynthFinder正式比較（[#66](https://github.com/kent-tokyo/renkin/issues/66)）— 固定500-targetサンプル・共有393化合物ストック・各ツールの設定下で、RENKIN Conservativeの`route_to_shared_stock`はAiZynthFinderより9.8ポイント高く（73/500 対 24/500、95% CI [7.0, 12.8]、exact McNemar p≈1.9e-11）。これはこのプロトコルに限定されたペア比較であり、一般的な探索能力の優位性を主張するものではありません。native構成はストックが揃わないため直接比較しません。
- [x] extracted template向けRing-context安全ガード（[#72](https://github.com/kent-tokyo/renkin/issues/72)/[#242](https://github.com/kent-tokyo/renkin/pull/242)）— opt-inの `--ring-context-policy`/`--ring-context-sidecar`。訓練データで環結合として一度も観測されていない環開閉切断のテンプレート誤適用を検出。デフォルトは引き続き `disabled`（既存挙動のまま）
- [x] `atom_economy` の100%への暗黙クランプを廃止（[#79](https://github.com/kent-tokyo/renkin/issues/79)）— ルートの精製物集合が対象の全質量を説明できない場合、新設の `atom_economy_status` フィールド（`normal`/`above_expected_range`/`not_evaluable`）で明示的に報告
- [x] Coverage mode（`--search-mode coverage`、[#101](https://github.com/kent-tokyo/renkin/issues/101)、v0.24.0で出荷）— opt-inのStage-1/Stage-2テンプレート数エスカレーション、下記candidate-generation coverage gapへの対応。500-target規模の一度限りのformal-TESTで確認済み（`data/coverage_mode_formal_test/protocol_v2.md`）：coverage +6.0pp、net gain +30、regression 0、reranker failure 0、Stage-2 timeout率0.25%——いずれも事前登録済み閾値に対して。出荷済み範囲は上記の特徴表参照

### 進行中

- [ ] Candidate-generation coverage gap — formal TESTコーパスの33.0%（1,618/4,903）がpositive candidateゼロで、これはrerankingでは原理的に解決できない天井。template-diversity-scalingは強いメカニズムであることを確認済み（Phase A.5/B.2、上記coverage mode参照）、higher-level-templateの研究方向はまだ未着手
- [ ] 5万テンプレートセット向けのtemplate retrieval index（element bitmask + bond-center prefilter）
- [ ] キャリブレーション済みroute confidence（`success_probability`を経験的solve rateへマッピング）

### 次

- [x] グラフルール拡張 — sulfonamide / carbamate cleavage（構造・原子収支ゲート付き、carbamateはv0.61.0で出荷）
- [x] urea cleavage — isocyanate + amineへの原子収支を検証したdefault ruleをローカル実装（次版候補、未公開）
- [ ] Stock-aware planning（価格・ハザード・入手性による再順位付け）
  - [x] exactなprivate stock候補に対する、価格・納期・入手性を考慮した
    決定論的なvendor offer選択
  - [x] ローカルcatalogの任意hazardラベルとblocked-hazard policy判定
  - [x] route単位のstock scoreと、複数route向け決定論的ランキングmetadata
  - [x] Constraint DSLの`max_route_cost`によるroute cost上限

<details>
<summary>過去のマイルストーン</summary>

以下のパーセント数値はその時点でのマイルストーンであり現在の性能ではない——一部は[現在の制約](#現在の制約)に記載のvalidator精度修正より前の数値で無効化済み。修正済みの過去計測値は[ベンチマーク](#ベンチマーク)参照。

- [x] 安定 `template_id`（`rule:<name>` / `smirks-sha256:<hex>`）+ `--template-metadata` evidence サイドカー + `renkin template ids`（[#41](https://github.com/kent-tokyo/renkin/issues/41) phase 1）
- [x] 基質固有の `examples`（`schema_version: 2`）— ステップごとに「exact substrate match」か「同一テンプレート・別基質」かを解決し、`--format explain` に表示、JSONでは `match_kind` フィールドとして提供（[#41](https://github.com/kent-tokyo/renkin/issues/41) phase 2）
- [x] 決定的なORD（Open Reaction Database）evidenceインポート — オフラインの `renkin evidence match`（exact-setバッチtemplate matcher）+ `scripts/ord_evidence_audit.py`（audit/converter）により `schema_version: 2` サイドカーへ変換（[#41](https://github.com/kent-tokyo/renkin/issues/41) phase 3A）
- [x] RETROSPECT 着想のオフライン候補リランキング基盤 — candidate proposal/selection の分離、feature schema v1、manifest v2、leakage-safe な train/val/test スプリット、baseline arm + 学習済み ranker arm、paired bootstrap + オフラインゲートツール（[#59](https://github.com/kent-tokyo/renkin/pull/59)）
- [x] `renkin-forward enumerate` — 既知反応物1つと明示的なpartnerライブラリからの、境界付きtemplate誘導型順反応列挙（[#64](https://github.com/kent-tokyo/renkin/issues/64)）
- [x] `renkin-forward hints` — partner不要の検索用ヒント（マッチしたテンプレートslot・不足partnerのSMARTS・結合デルタ）。具体的な生成物は予測しない（[#64](https://github.com/kent-tokyo/renkin/issues/64) phase 2）
- [x] `renkin-forward` CLI 強化 — バージョン管理された `ForwardPredictionReport`、決定的な候補ID/マージ/由来情報、reactant 順序に依存しないマッチング、厳格な CLI/route-JSON 検証
- [x] `apply_retro`/`run_reactants` 性能回帰の解消 — `chematic`を公開済み`0.8.0`（上流のautomorphism-orbit-pruned canonicalization、[chematic#193](https://github.com/kent-tokyo/chematic/pull/193)）へ移行。correctnessへの影響ゼロ
- [x] `renkin-bench --plausibility` — 順方向検証による妥当性レポート
- [x] `renkin-forward predict` — テンプレートベース順反応予測
- [x] `renkin-forward validate` — 逆合成ルートの順反応検証；stdin パイプ対応
- [x] `renkin-doctor` — 環境診断バイナリ（テンプレート・BB・Python・ツールバージョン）
- [x] 失敗時診断 — ルートゼロ時に `likely_causes` + `suggestions` の JSON ブロックを出力
- [x] `--format explain|compare|compare-json` — 人間可読・表形式ルート出力
- [x] `renkin stock stats|validate|coverage` — stock CSV 管理サブコマンド
- [x] Pareto 多目的探索 — `--format pareto`・`--objectives`・`find_pareto_routes` MCP ツール
- [x] 制約 DSL — `--constraints JSON`・`plan_with_constraints` MCP ツール
- [x] `renkin template stats|validate|dedup|explain|coverage` — テンプレート品質ツール
- [x] `renkin-kg` — 反応知識グラフ（分子↔反応 二部グラフ、GraphML/Cypher エクスポート）
- [x] MCP サーバー拡張 — 6 ツール体制（`explain_route`・`find_pareto_routes`・`plan_with_constraints` 追加）
- [x] ルートコストスコアリング — `route_cost` フィールド + `--bb-prices CSV` / `--stock stock.csv`
- [x] Cargo workspace 整備 — `crates/renkin-forward/` + `crates/renkin-kg/`
- [x] コア探索エンジンの基盤 — SMIRKS逆反応ルール+フラグメント正規化、A\*/AND-OR木探索（クローズドリスト・縮退ルートフィルタ付き）、SA Scoreヒューリスティック+ビームサーチ、`rayon`並列ルール適用（wasm32は逐次フォールバック）、FxHashMap/SmallVecビームフロンティア/SA Scoreメモ化/`Arc<PathNode>`パス共有等の性能最適化
- [x] マルチターゲット配布 — Pythonバインディング（PyO3+maturin、`pip install renkin`）、WASMビルド（`npm install renkin`）、crates.io/PyPI/npm公開+GitHub Actions CI/CD、WASMブラウザプレイグラウンド+i18n（EN/JA/ZH）
- [x] ベンチマークCLI（`renkin-bench`）+ USPTO-50k初期評価、MkDocsドキュメントサイト + GitHub Pagesプレイグラウンド
- [x] グラフベースビアリール切断 · O(1) canonical-SMILES BB インデックス
- [x] 四面体ステレオ @/@@ + E/Z 二重結合ステレオサポート
- [x] NNテンプレートスコアラー `--scorer` フラグ（tract-onnx、Pure Rust ONNX、C++依存なし）
- [x] 制約付き探索（`--avoid-elements` / `--require-elements`）+ `--verbose` 探索統計
- [x] **`#![forbid(unsafe_code)]`** — 全クレートで最初からコンパイラ保証の Pure Safe Rust

</details>

---

## 引用

学術論文で RENKIN を使用した場合は、[`CITATION.cff`](CITATION.cff)（正式な
バージョン管理付き引用レコード）を引用してください。GitHub の「Cite this
repository」ボタン（リポジトリページ上部）がこのファイルを直接読み込み、
APA / BibTeX 形式でのエクスポートに対応しています。

---

## セキュリティ

脆弱性は [GitHub プライベート脆弱性報告](https://github.com/kent-tokyo/renkin/security/advisories/new) からご報告ください。詳細は [SECURITY.md](SECURITY.md) を参照してください。

---

## ライセンス

MIT

---

*GitHub Topics: `retrosynthesis` `cheminformatics` `wasm` `rust` `drug-discovery` `casp` `synthesis-planning` `computational-chemistry`*

---

If RENKIN saves you time, a GitHub star helps others discover it.
