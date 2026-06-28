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

<p>
  <img alt="Pure Rust" src="https://img.shields.io/badge/Pure%20Rust-100%25-orange?logo=rust">
  <img alt="unsafe forbidden" src="https://img.shields.io/badge/unsafe-forbidden-success.svg">
  <img alt="WASM" src="https://img.shields.io/badge/WASM-ready-brightgreen">
  <img alt="PyO3" src="https://img.shields.io/badge/PyO3-Python%20bindings-blue">
  <img alt="MCP" src="https://img.shields.io/badge/MCP-ready-7f52ff">
  <img alt="templates" src="https://img.shields.io/badge/templates-up%20to%2050k-purple">
  <img alt="building blocks" src="https://img.shields.io/badge/building%20blocks-509-lightgrey">
  <img alt="USPTO-50k" src="https://img.shields.io/badge/USPTO--50k-78.1%25%20solved-brightgreen">
  <img alt="ChEMBL" src="https://img.shields.io/badge/ChEMBL-81.8%25%20solved-brightgreen">
</p>

[English README](./README.md) · [**ドキュメント**](https://kent-tokyo.github.io/renkin/) · [**ライブデモ →**](https://kent-tokyo.github.io/renkin/playground/)

---

## RENKINとは

RENKINは、目標分子（ゴール）から逆算して市販の安価な原料へと至る最適な化学反応経路を自動発見する**逆合成（Retrosynthesis）エンジン**です。**創薬・医薬化学・ケモインフォマティクス**において中心的な問題を解きます。

Rust言語と [`chematic`](https://docs.rs/chematic/) クレートで実装された純粋なRust製エンジン。C/C++依存ゼロ。全クレートに `#![forbid(unsafe_code)]` を適用し、コンパイラレベルで Pure Safe Rust を保証しています。

**[→ ライブプレイグラウンドを試す](https://kent-tokyo.github.io/renkin/playground/)** — ブラウザ上でWebAssemblyとして動作。インストール不要。  
**[→ ドキュメント全文](https://kent-tokyo.github.io/renkin/)** — APIリファレンス、使用例、ベンチマーク。

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

## インストール

```bash
pip install renkin          # Python
cargo add renkin            # Rust
npm install renkin          # JavaScript / Node.js
```

---

## クイックスタート

```python
import renkin

result = renkin.find_routes(
    "CC(=O)Oc1ccccc1C(=O)O",   # アスピリン
    depth=5,
    max_routes=3,
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

## 特徴

| 特徴 | 詳細 |
|---|---|
| **Pure Safe Rust** | 全クレートに `#![forbid(unsafe_code)]` — コンパイラ保証、C/C++依存ゼロ |
| **A\* / AND-OR木探索** | プラガブルフック付きRetro\*相当アルゴリズム（`MoleculeValueEstimator`, `ReactionPrior`） |
| **最大50k逆合成テンプレート** | USPTO-50k/MIT からrdchiralで自動抽出；頻度重み付け優先；`--templates` でカスタムセット対応 |
| **ルートスコアリング** | `confidence`, `step_confidence`, `success_probability`（Retro-prob方式）, `convergency`, `atom_economy` |
| **ルートコストスコアリング** | `route_cost = Σ(BB価格) + ステップ数×0.5`；`--bb-prices CSV` または `--stock stock.csv` で実価格対応 |
| **Pareto多目的探索** | `--format pareto` で `route_cost`・`success_probability`・`steps` 等のパレートフロントを返す；`--objectives` で目的関数をカスタム設定 |
| **制約 DSL** | `--constraints constraints.json` — JSON駆動の合成計画：元素フィルタ・ステップ数制限・信頼度閾値・優先反応族；LLM → RENKIN パイプラインに対応 |
| **出力フォーマット** | `--format json` · `tree` · `mermaid` · `explain`（ルートごとの人間可読解説）· `compare`（並列比較表）· `compare-json` · `pareto` |
| **失敗時診断** | ルートが見つからない場合、JSON に `diagnostics` ブロック（`likely_causes` + `suggestions`）を付加 |
| **順方向検証** | `renkin-forward validate` で各ステップを順方向適用して検証；stdin パイプ対応 |
| **妥当性レポート** | `renkin-bench --plausibility` — ベストルートを順方向検証し、複合妥当性スコアを算出 |
| **PaRoutesベンチマーク** | `renkin-bench --input-format paroutes` でmulti-step ground-truth評価（`depth_delta`, `route_diversity`） |
| **原子収支チェック** | `renkin-bench` で `target_MW > Σ precursor_MW` のステップを検出（CompleteRXN参照） |
| **stock CSV 管理** | `renkin stock stats\|validate\|coverage` — SMILES・名称・ベンダー・価格・ハザード情報を持つ stock CSV を検査 |
| **テンプレート品質ツール** | `renkin template stats\|validate\|dedup\|explain\|coverage` — テンプレートセットの頻度分布・有効性・重複・検索・カバレッジを検査 |
| **MCPサーバー** | `renkin-mcp` が 6 ツールを提供：`find_routes`, `validate_route`, `explain_route`, `find_pareto_routes`, `plan_with_constraints`, `estimate_diversity` |
| **`renkin-doctor`** | 環境診断バイナリ — テンプレート・市販品データ・Python インポート・ツールバージョンを検査 |
| **`renkin-kg`** | 反応知識グラフ構築ツール — ルートから分子↔反応の二部グラフを生成；GraphML / Cypher 形式でエクスポート |
| **ビームサーチ** | `--beam-width N` でメモリ制約付き探索；`SmallVec<[FEntry; 6]>` スタック割り当て |
| **並列ルール適用** | 非WASM環境で `rayon`；wasm32 はシーケンシャルフォールバック |
| **tract-onnx NNスコアラー** | Pure Rust ONNXインファレンス（C++依存なし） — Phase B テンプレート関連性スコアリングの `--scorer` フラグ |
| **手順ヒント** | 19件の手工芸ルールに `procedure_hint` — QFANG方式手順生成の受け口 |
| **四面体ステレオ @/@@** | chematic 0.4.16 による完全な立体化学サポート |
| **Python** | `pip install renkin` — Linux/macOS/Windows プリビルドwheels |
| **WASM** | ~500 KB バンドル — ブラウザでネイティブに近い速度で動作 |
| **509件の市販原料** | アリールハライド、ボロン酸、ヘテロ環、医薬品アミン、アミノ酸 |

---

## ベンチマーク

USPTO-50kテストセット（全4,907分子評価）:

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
| **+ diaryl sulfone ルール、509 BB** | **3831/4907** | **78.1%** | **509** | **5,000** | **5** | **100** | **≈2,800** |

\* 29/50チャンク、旧バイナリ  
† 全50チャンク完了 — **72.1%**（3,540/4,907）確認済

USPTO-50k 標準ベンチマーク（多段階経路探索、同 train/test 分割）において、RENKIN（**78.1%**）は AiZynthFinder（45–53%）・Retro\*（44.3%）・ASKCOS（41%）の論文値を数値上回る。ただしこれらは 2019–2020 年の論文値で BB 数・テンプレート数等の条件が異なり、matched-condition 実験は未実施。  
*注意: LocalRetro（53.4%）・GLG（58.0%）は単ステップ top-1 予測精度であり、多段階経路探索成功率とは別の指標のため直接比較不可。*  
[ベンチマーク詳細 →](https://kent-tokyo.github.io/renkin/benchmark/)

> **ベンチマーク範囲に関する注意**: USPTO-50k はここでは *標準化されたサニティベンチマーク* として使用しており、実世界の広範な合成性能を証明するものではありません。同コーパスは主に製薬合成で一般的な C–C・C–N 結合形成に偏っており、USPTO の掲載が少ない反応タイプは体系的に不利になります。ChEMBL 承認薬（OOD）での **81.8%**（409/500）はルールセットがテストコーパスを超えて汎化することを示唆しますが、任意のターゲットに対する経路品質を保証するものではありません。

---

## 競合比較

| ツール | 言語 | ライセンス | WASM | ゼロ依存 | アルゴリズム | テンプレート | 在庫 |
|---|---|---|---|---|---|---|---|
| **ASKCOS** | Python | CC BY-NC | No | No（Docker/64GB） | MCTS + A\* | USPTO（ML） | ZINC |
| **AiZynthFinder** | Python | MIT | No | No（conda+モデル） | MCTS | USPTO（ML/~50k） | eMolecules（~6M） |
| **SYNTHIA** | クローズド | 独自 | No | No | SMARTS+AND/OR | 手動作成 | Sigma-Aldrich |
| **IBM RXN** | クローズド | SaaS | No | No | Transformer | USPTO | — |
| **Retro\*** | Python | MIT | No | No（未メンテ） | A\*+AND/OR | USPTO（ML） | eMolecules |
| **★ RENKIN** | **Rust** | **MIT** | **Yes** | **Yes** | **A\*+AND/OR** | 手動+rdchiral（5k デフォルト；`--templates` で50k対応） | 509+（拡張可） |

**RENKINの目標**: GPU なし・学習データなし・ブラックボックスなし——キュレーション済みルールと自動抽出テンプレートだけで、ニューラルネットベースのツールに匹敵する精度を目指す。USPTO-50k 標準ベンチマーク（全ツール共通の train/test 分割）で **78.1%**（3,831/4,907 — 全件確認済）を達成。5,000件のテンプレートと509件の市販原料、テンプレート頻度重み付け（Phase A）——AiZynthFinder の NN テンプレートスコアリングと同原理——の組み合わせによる成果。そしてブラウザ・CLI・Python、どこでも動く。

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
| `plan_with_constraints` | 制約 DSL による合成計画（元素フィルタ・ステップ数・信頼度閾値） |
| `estimate_diversity` | ルート多様性・カバレッジ指標 |

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
│  renkin（逆合成）                 renkin-forward（開発予定）       │
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
│   ├── main.rs                  # CLI バイナリ（--templates, --scorer, --constraints, --objectives フラグ対応）
│   ├── bin/benchmark.rs         # renkin-bench バイナリ（--plausibility フラグ対応）
│   ├── bin/doctor.rs            # renkin-doctor 環境診断バイナリ
│   ├── bin/fp.rs                # renkin-fp ECFP4 フィンガープリント（nn-scoring フィーチャー）
│   ├── bin/mcp.rs               # renkin-mcp MCP サーバー（6 ツール）
│   ├── chem_env.rs              # 逆合成ルール・市販品判定・テンプレートローダー
│   ├── score.rs                 # SA Score ヒューリスティック
│   ├── search.rs                # A* / AND-OR 木探索エンジン
│   ├── scorer.rs                # Phase B: tract-onnx NNテンプレートスコアラー
│   ├── python.rs                # PyO3 バインディング
│   └── wasm.rs                  # wasm-bindgen バインディング
├── crates/                      ← 兄弟クレート
│   ├── renkin-forward/          # 順反応予測（reactants → products）
│   └── renkin-kg/               # 反応知識グラフ（分子↔反応 二部グラフ、GraphML/Cypher エクスポート）
├── data/
│   ├── building_blocks.smi              # 509件の市販原料（キュレーション済み）
│   ├── templates_extracted_5000.smi     # 5,000件の自動抽出SMIRKSテンプレート
│   ├── benchmark_targets.smi            # 内部ベンチマークセット
│   └── bench_chunks/                    # USPTO-50k チャンク別結果
├── scripts/
│   ├── extract_templates.py         # rdchiral テンプレート抽出パイプライン
│   └── run_benchmark_chunks.sh      # 再開可能チャンクベンチマーク
├── docs/                # MkDocs ソース → kent-tokyo.github.io/renkin/
└── mkdocs.yml
```

---

## ロードマップ

- [x] **Phase 1** — SMIRKS 逆反応ルール + フラグメント正規化
- [x] **Phase 2** — A\* / AND-OR 木探索・クローズドリスト・縮退ルートフィルタ
- [x] **Phase 3** — SA Score ヒューリスティック + ビームサーチ
- [x] **Phase 4** — 並列ルール適用（rayon; WASM では逐次フォールバック）
- [x] **Phase 5** — Python バインディング（PyO3 + maturin）· `pip install renkin`
- [x] **Phase 6** — WASM ビルド · `npm install renkin`
- [x] **Phase 7** — ベンチマーク CLI（`renkin-bench`）+ USPTO-50k 初期評価
- [x] **Phase 8** — ユニットテスト · ルール → 31件 · 市販原料 → 463件
- [x] **Phase 9** — WASM ブラウザプレイグラウンド + i18n（EN/JA/ZH）
- [x] **Phase 10** — グラフベースビアリール切断 · O(1) canonical-SMILES BB インデックス
- [x] **Phase 11** — crates.io / PyPI / npm 公開 · GitHub Actions CI/CD
- [x] **Phase 12** — MkDocs ドキュメントサイト · GitHub Pages プレイグラウンド
- [x] **Phase 13** — 正式 USPTO-50k ベンチマーク: **7.5%**（depth=3、31ルール）
- [x] **Phase 14** — 自動テンプレート抽出（rdchiral）: **27.8%**（depth=3、222ルール）
- [x] **Phase 15** — 四面体ステレオ @/@@ サポート（chematic 0.4.16）✅
- [x] **Phase 15a** — E/Z 二重結合ステレオフィルタ有効化（chematic-rxn 0.4.15 / issue #21）
- [x] **Phase 17** — chematic 0.4.12: Bug #13（BFSリーク）+ Bug #14（canonical SMILES）修正確認
- [x] **Phase 18** — テンプレート頻度重み付け（Phase A）: **72.1%** USPTO-50k（3,540/4,907 — 全件確認 ✅）
- [x] **Phase 19** — Rust エンジン内部最適化（split_fragments・is_bb・元素プリスクリーニング）
- [x] **Phase 20** — FxHashMap/FxHashSet（rustc-hash）で標準コレクション全体を置き換え
- [x] **Phase 21** — SmallVec<[FEntry; 6]> ビームフロンティア（スタック割り当て）
- [x] **Phase 22** — SA Score メモ化キャッシュ（探索ごと）
- [x] **Phase 23** — Arc<PathNode> 永続連結リストによるパス共有（子ノードあたり O(1)）
- [x] **Phase 24** — apply_retro メモ化キャッシュ（重複中間体の SMARTS VF2 スキップ）
- [x] **Phase 25** — 5,000テンプレート + 480 BB: **78.0%** USPTO-50k（3,826/4,907 ✅、2,775 ms/mol）
- [x] **Phase 26** — diaryl sulfone retro ルール + 509 BB: **78.1%** USPTO-50k（3,831/4,907 ✅）
- [x] **Phase B** — NNテンプレートスコアラー `--scorer` フラグ（tract-onnx、Pure Rust ONNX、C++依存なし）✅
- [x] **制約付き探索** — `--avoid-elements` / `--require-elements` / `--verbose`
- [x] **`#![forbid(unsafe_code)]`** — 全クレートでコンパイラ保証の Pure Safe Rust
- [x] ルートコストスコアリング — `route_cost` フィールド + `--bb-prices CSV` / `--stock stock.csv`
- [x] Cargo workspace 整備 — `crates/renkin-forward/` + `crates/renkin-kg/`
- [x] `renkin-forward predict` — テンプレートベース順反応予測 ✅
- [x] `renkin-forward validate` — 逆合成ルートの順反応検証；stdin パイプ対応 ✅
- [x] `renkin-doctor` — 環境診断バイナリ（テンプレート・BB・Python・ツールバージョン）
- [x] 失敗時診断 — ルートゼロ時に `likely_causes` + `suggestions` の JSON ブロックを出力
- [x] `--format explain|compare|compare-json` — 人間可読・表形式ルート出力
- [x] `renkin stock stats|validate|coverage` — stock CSV 管理サブコマンド
- [x] Pareto 多目的探索 — `--format pareto`・`--objectives`・`find_pareto_routes` MCP ツール
- [x] 制約 DSL — `--constraints JSON`・`plan_with_constraints` MCP ツール
- [x] `renkin-bench --plausibility` — 順方向検証による妥当性レポート
- [x] `renkin template stats|validate|dedup|explain|coverage` — テンプレート品質ツール
- [x] `renkin-kg` — 反応知識グラフ（分子↔反応 二部グラフ、GraphML/Cypher エクスポート）
- [x] MCP サーバー拡張 — 6 ツール体制（`explain_route`・`find_pareto_routes`・`plan_with_constraints` 追加）

---

## 引用

学術論文で RENKIN を使用した場合は以下を引用してください：

```bibtex
@software{renkin2026,
  author    = {kent-tokyo},
  title     = {{RENKIN}: Retrosynthesis Engine for Knowledge-Informed Navigation},
  year      = {2026},
  url       = {https://github.com/kent-tokyo/renkin/releases/tag/v0.15.5},
  version   = {0.15.5},
  license   = {MIT}
}
```

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
