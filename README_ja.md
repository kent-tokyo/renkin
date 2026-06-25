# RENKIN — **R**etrosynthetic **E**xploration **N**etwork for **K**nowledge-**I**nformed **N**avigation

> **コンピュータ支援合成計画（CASP）· Pure Rust · WebAssembly · Python**  
> 錬金（れんきん）― 錬金術のように、目標分子を安価な原料へと逆変換する。

[![CI](https://github.com/kent-tokyo/renkin/actions/workflows/ci.yml/badge.svg)](https://github.com/kent-tokyo/renkin/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/renkin)](https://crates.io/crates/renkin)
[![PyPI](https://img.shields.io/pypi/v/renkin)](https://pypi.org/project/renkin/)
[![npm](https://img.shields.io/npm/v/renkin)](https://www.npmjs.com/package/renkin)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
[![WASM](https://img.shields.io/badge/WASM-ready-brightgreen)](https://kent-tokyo.github.io/renkin/playground/)
[![Pure Rust](https://img.shields.io/badge/Pure-Rust-orange?logo=rust)](https://www.rust-lang.org)
[![unsafe forbidden](https://img.shields.io/badge/unsafe-forbidden-success.svg)](https://github.com/rust-secure-code/safety-dance/)
[![Open In Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/github/kent-tokyo/renkin/blob/master/examples/renkin_quickstart.ipynb)

[English README](./README.md) · [**ドキュメント**](https://kent-tokyo.github.io/renkin/) · [**ライブデモ →**](https://kent-tokyo.github.io/renkin/playground/)

---

## RENKINとは

RENKINは、目標分子（ゴール）から逆算して市販の安価な原料へと至る最適な化学反応経路を自動発見する**逆合成（Retrosynthesis）エンジン**です。**創薬・医薬化学・ケモインフォマティクス**において中心的な問題を解きます。

Rust言語と [`chematic`](https://docs.rs/chematic/) クレートで実装された純粋なRust製エンジン。C/C++依存ゼロ。全クレートに `#![forbid(unsafe_code)]` を適用し、コンパイラレベルで Pure Safe Rust を保証しています。

**[→ ライブプレイグラウンドを試す](https://kent-tokyo.github.io/renkin/playground/)** — ブラウザ上でWebAssemblyとして動作。インストール不要。  
**[→ ドキュメント全文](https://kent-tokyo.github.io/renkin/)** — APIリファレンス、使用例、ベンチマーク。

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
| **A\* / AND-OR木探索** | MCTSより探索効率が高いRetro\*相当アルゴリズム |
| **SA Scoreヒューリスティック** | h = Σ(1 + 0.5·(sa−1)/9)、アドミッシブル性を維持 |
| **SA Scoreメモ化キャッシュ** | 探索ごとにキャッシュ — 重複中間体での再計算を省略 |
| **ビームサーチ** | `--beam-width N` でメモリ制約付き探索；フロンティアに `SmallVec<[FEntry; 6]>` スタック割り当て |
| **5,000件の逆合成テンプレート** | USPTO-50k訓練セットからrdchiralで自動抽出；頻度重み付けビーム優先 |
| **テンプレート頻度重み付け** | Phase A: 訓練セット頻度 `weight = ln(count+1)` → ビームサーチで高頻度テンプレート優先（+19pp） |
| **元素プリスクリーニング** | `required_elements` bitset でSMARTSマッチング前に不適合ルールを除外 |
| **apply_retroメモ化** | 重複中間体のSMARTS VF2をスキップ — 探索ごとのキャッシュ |
| **Arc<PathNode> パス共有** | 永続連結リスト；子ノードあたりO(1)（O(depth)クローン不要） |
| **FxHashMap / FxHashSet** | rustc-hash で標準コレクション全体を置き換え、高速ハッシュ |
| **自動テンプレート抽出** | `scripts/extract_templates.py` — rdchiral + chematic互換フィルタ |
| **グラフベースAr-Ar切断** | ブリッジボンドDFS — 対称ビアリールを正確に処理 |
| **並列ルール適用** | `rayon` で並列評価（WASM では逐次フォールバック） |
| **tract-onnx NNスコアラー** | Pure Rust ONNXインファレンス（C++依存なし） — Phase B テンプレート関連性スコアリングの `--scorer` フラグ |
| **ルート可視化** | `--format tree` ASCII木 · `--format mermaid` GitHub/Notion対応フローチャート |
| **制約付き探索** | `--avoid-elements "Br,I"` で禁止元素を除外 · `--require-elements "B"` で必須元素を指定 |
| **探索トレース** | `--verbose` で展開ノード数・経過時間をstderrに出力（stdout出力は無影響） |
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

---

## 競合比較

| ツール | 言語 | ライセンス | WASM | ゼロ依存 | アルゴリズム | テンプレート | 在庫 |
|---|---|---|---|---|---|---|---|
| **ASKCOS** | Python | CC BY-NC | No | No（Docker/64GB） | MCTS + A\* | USPTO（ML） | ZINC |
| **AiZynthFinder** | Python | MIT | No | No（conda+モデル） | MCTS | USPTO（ML/~50k） | eMolecules（~6M） |
| **SYNTHIA** | クローズド | 独自 | No | No | SMARTS+AND/OR | 手動作成 | Sigma-Aldrich |
| **IBM RXN** | クローズド | SaaS | No | No | Transformer | USPTO | — |
| **Retro\*** | Python | MIT | No | No（未メンテ） | A\*+AND/OR | USPTO（ML） | eMolecules |
| **★ RENKIN** | **Rust** | **MIT** | **Yes** | **Yes** | **A\*+AND/OR** | 手動+rdchiral（5,000件） | 509+（拡張可） |

**RENKINの目標**: GPU なし・学習データなし・ブラックボックスなし——キュレーション済みルールと自動抽出テンプレートだけで、ニューラルネットベースのツールに匹敵する精度を目指す。USPTO-50k 標準ベンチマーク（全ツール共通の train/test 分割）で **78.1%**（3,831/4,907 — 全件確認済）を達成。5,000件のテンプレートと509件の市販原料、テンプレート頻度重み付け（Phase A）——AiZynthFinder の NN テンプレートスコアリングと同原理——の組み合わせによる成果。そしてブラウザ・CLI・Python、どこでも動く。

---

## アーキテクチャ

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
renkin/
├── Cargo.toml
├── src/
│   ├── lib.rs               # ライブラリクレート
│   ├── main.rs              # CLI バイナリ（--templates、--scorer フラグ対応）
│   ├── bin/benchmark.rs     # renkin-bench バイナリ（--templates フラグ対応）
│   ├── chem_env.rs          # 5,000逆合成ルール・市販品判定・テンプレートローダー
│   ├── score.rs             # SA Score ヒューリスティック
│   ├── search.rs            # A* / AND-OR 木探索エンジン
│   ├── scorer.rs            # Phase B: tract-onnx NNテンプレートスコアラー
│   ├── python.rs            # PyO3 バインディング
│   └── wasm.rs              # wasm-bindgen バインディング
├── data/
│   ├── building_blocks.smi              # 480件の市販原料（キュレーション済み）
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
- [x] **Phase 26** — `--format tree|mermaid` ルート可視化 + JSON に `score` フィールド追加
- [x] **制約付き探索** — `--avoid-elements` / `--require-elements` / `--verbose`
- [x] **`#![forbid(unsafe_code)]`** — 全クレートでコンパイラ保証の Pure Safe Rust

---

## 引用

学術論文で RENKIN を使用した場合は以下を引用してください：

```bibtex
@software{renkin2026,
  author    = {kent-tokyo},
  title     = {{RENKIN}: Retrosynthetic Exploration Network for Knowledge-Informed Navigation},
  year      = {2026},
  url       = {https://github.com/kent-tokyo/renkin},
  version   = {0.1.5},
  license   = {MIT}
}
```

---

## ライセンス

MIT

---

*GitHub Topics: `retrosynthesis` `cheminformatics` `wasm` `rust` `drug-discovery` `casp` `synthesis-planning` `computational-chemistry`*
