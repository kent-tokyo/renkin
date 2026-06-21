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

[English README](./README.md) · [**ドキュメント**](https://kent-tokyo.github.io/renkin/) · [**ライブデモ →**](https://kent-tokyo.github.io/renkin/playground/)

---

## RENKINとは

RENKINは、目標分子（ゴール）から逆算して市販の安価な原料へと至る最適な化学反応経路を自動発見する**逆合成（Retrosynthesis）エンジン**です。**創薬・医薬化学・ケモインフォマティクス**において中心的な問題を解きます。

Rust言語と [`chematic`](https://docs.rs/chematic/) クレートで実装された純粋なRust製エンジン。C/C++依存ゼロ。

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
    --templates data/templates_extracted.smi
```

---

## 特徴

| 特徴 | 詳細 |
|---|---|
| **Pure Rust** | C/C++依存ゼロ。`cargo build` のみで全プラットフォームに対応 |
| **A\* / AND-OR木探索** | MCTSより探索効率が高いRetro\*相当アルゴリズム |
| **SA Scoreヒューリスティック** | h = Σ(1 + 0.5·(sa−1)/9)、アドミッシブル性を維持 |
| **ビームサーチ** | `--beam-width N` でメモリ制約付き探索 |
| **314件の逆合成ルール** | 手書き 31件 + rdchiral 自動抽出 283件 |
| **テンプレート頻度重み付け** | Phase A: 訓練セット頻度 `weight = ln(count+1)` → ビームサーチで高頻度テンプレート優先（+19pp） |
| **元素プリスクリーニング** | `required_elements` bitset でSMARTSマッチング前に不適合ルールを除外 |
| **自動テンプレート抽出** | `scripts/extract_templates.py` — rdchiral + chematic互換フィルタ |
| **グラフベースAr-Ar切断** | ブリッジボンドDFS — 対称ビアリールを正確に処理 |
| **並列ルール適用** | `rayon` で並列評価（WASM では逐次フォールバック） |
| **Python** | `pip install renkin` — Linux/macOS/Windows プリビルドwheels |
| **WASM** | ~500 KB バンドル — ブラウザでネイティブに近い速度で動作 |
| **463件の市販原料** | アリールハライド、ボロン酸、ヘテロ環、医薬品アミン、アミノ酸 |

---

## ベンチマーク

USPTO-50kテストセット（全4,907分子評価）:

> **評価条件の注記**: 全数値は USPTO-50k の標準 train/test 分割（同一コーパス）を使用。テンプレートは訓練セットから抽出しテストセットで評価——AiZynthFinder 等の論文と同じ手法。数値は USPTO-50k ドメイン内での性能を示すものであり、分布外（OOD）汎化性は別途検証が必要。

| 設定 | 解決数 | 解決率 | BB数 | ルール数 | depth | beam |
|---|---|---|---|---|---|---|
| v0.1.0 初期 | 366/4907 | 7.5% | 463 | 31 | 3 | 50 |
| 自動テンプレート追加（top-300） | 1363/4907 | 27.8% | 463 | 222 | 3 | 50 |
| depth=5 + top-500 テンプレート | 2315/4907 | 47.2% | 463 | 314 | 5 | 50 |
| + beam=100 | 2688/4907 | 54.8%* | 463 | 314 | 5 | 100 |
| + Phase A（頻度重み付け） | **~3581/4907** | **~73%†** | 463 | 314 | 5 | 100 |

\* 29/50チャンク、旧バイナリ  
† 32/50チャンク確認値（2,263/3,100）；全件ベンチマーク実行中

USPTO-50k 標準ベンチマーク（同 train/test 分割）において、AiZynthFinder（45–53%）・Retro\*（44.3%）・ASKCOS（41%）・LocalRetro（53.4%）・GLG（58.0%）を上回る。  
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
| **★ RENKIN** | **Rust** | **MIT** | **Yes** | **Yes** | **A\*+AND/OR** | 手動+rdchiral（314件） | 463+（拡張可） |

**RENKINの目標**: GPU なし・学習データなし・ブラックボックスなし——キュレーション済みルールと自動抽出テンプレートだけで、ニューラルネットベースのツールに匹敵する精度を目指す。USPTO-50k 標準ベンチマーク（全ツール共通の train/test 分割）で **~73%**（32/50チャンク確認値）を達成し、AiZynthFinder（45–53%）・LocalRetro（53.4%）・GLG（58.0%）を上回る。テンプレート頻度重み付け（Phase A）——AiZynthFinder の NN テンプレートスコアリングと同原理——が均等重み付けより +19pp の向上をもたらす。そしてブラウザ・CLI・Python、どこでも動く。

---

## アーキテクチャ

```
目標 SMILES
     │
     ▼
┌─────────────────────────┐
│     chem_env.rs         │  ← chematic ラッパー
│  - SMILES パース        │     canonical-SMILES HashSet BB照合（O(1)）
│  - 314 逆反応ルール     │     フラグメント正規化・リークフィルタ
│  - 市販品チェック       │     小規模セット向け VF2 フォールバック
└────────────┬────────────┘
             │  par_iter (rayon / WASM では逐次)
             ▼
┌─────────────────────────┐
│      search.rs          │  ← A* / AND-OR 木探索
│  - 優先度付きキュー     │     SA Score ヒューリスティック
│  - クローズドリスト     │     ビームサーチ枝刈り
│  - 縮退ルートフィルタ   │
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
  JSON  ←  CLI / Python / WASM
```

---

## プロジェクト構成

```
renkin/
├── Cargo.toml
├── src/
│   ├── lib.rs               # ライブラリクレート
│   ├── main.rs              # CLI バイナリ（--templates フラグ対応）
│   ├── bin/benchmark.rs     # renkin-bench バイナリ（--templates フラグ対応）
│   ├── chem_env.rs          # 314逆合成ルール・市販品判定・テンプレートローダー
│   ├── score.rs             # SA Score ヒューリスティック
│   ├── search.rs            # A* / AND-OR 木探索エンジン
│   ├── python.rs            # PyO3 バインディング
│   └── wasm.rs              # wasm-bindgen バインディング
├── data/
│   ├── building_blocks.smi          # 463件の市販原料（キュレーション済み）
│   ├── templates_extracted.smi      # 283件の自動抽出SMIRKSテンプレート（top-500）
│   ├── benchmark_targets.smi        # 内部ベンチマークセット
│   └── bench_chunks/                # USPTO-50k チャンク別結果
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
- [x] **Phase 17** — chematic 0.4.12: Bug #13（BFSリーク）+ Bug #14（canonical SMILES）修正確認
- [x] **Phase 18** — テンプレート頻度重み付け（Phase A）: **~73%** USPTO-50k（32/50チャンク確認）
- [x] **Phase 19** — Rust エンジン内部最適化（split_fragments・is_bb・元素プリスクリーニング）
- [ ] **Phase 15** — 立体化学対応（CIP SMIRKS）
- [ ] **Phase 16** — 大規模市販原料DB連携
- [ ] **Phase B** — ONNX テンプレート関連性モデル（分子固有テンプレート選択）

---

## ライセンス

MIT
