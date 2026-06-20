# RENKIN — 逆合成エンジン

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
./target/release/renkin --target "CC(=O)Oc1ccccc1C(=O)O" --depth 5
```

---

## 特徴

| 特徴 | 詳細 |
|---|---|
| **Pure Rust** | C/C++依存ゼロ。`cargo build` のみで全プラットフォームに対応 |
| **A\* / AND-OR木探索** | MCTSより探索効率が高いRetro\*相当アルゴリズム |
| **SA Scoreヒューリスティック** | h = Σ(1 + 0.5·(sa−1)/9)、アドミッシブル性を維持 |
| **ビームサーチ** | `--beam-width N` でメモリ制約付き探索 |
| **グラフベースAr-Ar切断** | ブリッジボンドDFS — 対称ビアリールを正確に処理 |
| **並列ルール適用** | `rayon` で並列評価（WASM では逐次フォールバック） |
| **Python** | `pip install renkin` — Linux/macOS/Windows プリビルドwheels |
| **WASM** | 493 KB バンドル — ブラウザでネイティブに近い速度で動作 |
| **480件超の市販原料** | アリールハライド、ボロン酸、ヘテロ環、医薬品アミン、アミノ酸 |
| **20件の反応ルール** | 下表参照 |

---

## 逆合成ルール（全20件）

| ルール名 | 反応タイプ | 実装方式 |
|---|---|---|
| `ester_cleavage` | エステル → 酸 + アルコール | SMIRKS |
| `amide_cleavage` | アミド → 酸 + アミン | SMIRKS |
| `friedel_crafts_acylation_retro` | Ar-C(=O)R → Ar-H + 酸クロリド | SMIRKS |
| `aryl_carboxylation_retro` | Ar-COOH → Ar-H + CO₂ | SMIRKS |
| `aryl_amine_retro` | Ar-N → Ar-H + アミン | SMIRKS |
| `buchwald_hartwig_retro` | Ar-N → Ar-Br + アミン | SMIRKS |
| `aryl_ether_retro` | Ar-O → Ar-OH + フラグメント | SMIRKS |
| `aryl_chloride_retro` | Ar-Cl → Ar-H（retro-SNAr / Pd C-Cl） | SMIRKS |
| `aryl_iodide_retro` | Ar-I → Ar-H（retro-Pd/Cu C-I） | SMIRKS |
| `aryl_fluoride_snAr_retro` | Ar-F → Ar-H（retro-SNAr） | SMIRKS |
| `aryl_chloride_to_bromide` | Ar-Cl → Ar-Br（ハロゲン交換） | SMIRKS |
| `suzuki_retro` | Ar-Ar → Ar-Br + Ar-H | **グラフ**（ブリッジボンドDFS） |
| `heck_retro` | Ar-CH=CH-R → Ar-Br + ビニル | SMIRKS |
| `negishi_retro` | Ar-CH₂ → Ar-Br + アルキル | SMIRKS |
| `cc_single_cleavage` | C–C → 2フラグメント | SMIRKS |
| `wittig_retro` | C=C → C=O + C=O | SMIRKS |
| `reductive_amination_retro` | C–N → C=O + アミン | SMIRKS |
| `cn_aliphatic_cleavage` | C–N → 2フラグメント | SMIRKS |
| `co_aliphatic_cleavage` | C–O → 2フラグメント | SMIRKS |
| `alcohol_oxidation_retro` | C–OH → C=O | SMIRKS |

---

## ベンチマーク

USPTO-50kテストセット（500分子サンプル）:

| 設定 | 解決数 | 解決率 | BB数 | ルール数 |
|---|---|---|---|---|
| v0.1.0（depth=2, beam=20） | 13/500 | 2.6% | 277 | 14 |
| 現在（depth=2, beam=20） | **25/500** | **5.0%** | **480+** | **20** |

**79 ms/分子**（Apple Mシリーズ、シングルスレッド）。[ベンチマーク詳細 →](https://kent-tokyo.github.io/renkin/benchmark/)

---

## 競合比較

| ツール | 言語 | ライセンス | WASM | ゼロ依存 | アルゴリズム | テンプレート | 在庫 |
|---|---|---|---|---|---|---|---|
| **ASKCOS** | Python | CC BY-NC | No | No（Docker/64GB） | MCTS + A\* | USPTO（ML） | ZINC |
| **AiZynthFinder** | Python | MIT | No | No（conda+モデル） | MCTS | USPTO（ML/~50k） | eMolecules（~6M） |
| **SYNTHIA** | クローズド | 独自 | No | No | SMARTS+AND/OR | 手動作成 | Sigma-Aldrich |
| **IBM RXN** | クローズド | SaaS | No | No | Transformer | USPTO | — |
| **Retro\*** | Python | MIT | No | No（未メンテ） | A\*+AND/OR | USPTO（ML） | eMolecules |
| **MEGAN** | Python | MIT | No | No（PyTorch） | グラフTransformer | USPTO | — |
| **★ RENKIN** | **Rust** | **MIT** | **Yes** | **Yes** | **A\*+AND/OR** | 手動作成（20件） | 480+（拡張可） |

**RENKINのポジション**: Docker/condaが使えない環境、ブラウザ/エッジデプロイが必要な場面で威力を発揮する、ポータブル・組み込み可能なCASPエンジン。USPTOリコール率最大化ではなく**デプロイしやすさ**を優先。

---

## アーキテクチャ

```
目標 SMILES
     │
     ▼
┌─────────────────────────┐
│     chem_env.rs         │  ← chematic ラッパー
│  - SMILES パース        │     SMARTS VF2 市販品判定
│  - 20 SMIRKS 逆反応適用 │     フラグメント正規化
│  - 市販品チェック       │     HashMap O(1) 前絞り込み
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
├── CHANGELOG.md
├── src/
│   ├── lib.rs           # ライブラリクレート
│   ├── main.rs          # CLI バイナリ
│   ├── bin/benchmark.rs # renkin-bench バイナリ
│   ├── chem_env.rs      # 20逆合成ルール・市販品判定
│   ├── score.rs         # SA Score ヒューリスティック
│   ├── search.rs        # A* / AND-OR 木探索エンジン
│   ├── python.rs        # PyO3 バインディング
│   └── wasm.rs          # wasm-bindgen バインディング
├── data/
│   ├── building_blocks.smi      # 480件超の市販原料
│   ├── benchmark_targets.smi   # 42分子内部ベンチマークセット
│   └── uspto50k_benchmark_result.json
├── demo/index.html      # ローカル WASM デモ
├── docs/                # MkDocs ソース → kent-tokyo.github.io/renkin/
│   ├── index.md
│   ├── getting_started/
│   ├── api/
│   ├── examples/
│   ├── benchmark.md
│   └── playground/index.html
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
- [x] **Phase 8** — ユニットテスト23件 · ルール 5→20件 · 市販原料 30→480件超
- [x] **Phase 9** — WASM ブラウザプレイグラウンド + 内部ベンチマーク（42分子）
- [x] **Phase 10** — グラフベースビアリール切断 · O(1) BB HashMap インデックス
- [x] **Phase 11** — crates.io / PyPI / npm 公開 · GitHub Actions CI/CD
- [x] **Phase 12** — MkDocs ドキュメントサイト · GitHub Pages プレイグラウンド
- [ ] **Phase 13** — AiZynthFinder / Retro\* との正式 USPTO-50k 比較
- [ ] **Phase 14** — USPTO-50k 訓練セットからの自動テンプレート抽出（rdchiral）
- [ ] **Phase 15** — 立体化学対応（CIP SMIRKS）
- [ ] **Phase 16** — 大規模市販原料DB（eMolecules / ZINC 連携）
- [ ] **Phase 17** — chematic upstream 修正対応（#13 BFS リーク, #14 canonical SMILES）

---

## ライセンス

MIT
