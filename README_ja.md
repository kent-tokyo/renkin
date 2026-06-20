# RENKIN — 逆合成エンジン

> **コンピュータ支援合成計画（CASP）· Pure Rust · WebAssembly · Python**  
> 錬金（れんきん）― 錬金術のように、目標分子を安価な原料へと逆変換する。  
> **超快適・軽量・高速**な逆合成エンジン。

[![Crates.io](https://img.shields.io/crates/v/renkin)](https://crates.io/crates/renkin)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
[![WASM](https://img.shields.io/badge/WASM-ready-brightgreen)](https://github.com/kent-tokyo/renkin/tree/master/demo)
[![Pure Rust](https://img.shields.io/badge/Pure-Rust-orange?logo=rust)](https://www.rust-lang.org)

[English README](./README.md)

---

## RENKINとは

RENKINは、目標分子（ゴール）から逆算して市販の安価な原料へと至る最適な化学反応経路を自動発見する**逆合成（Retrosynthesis）エンジン**です。**創薬・医薬化学・ケモインフォマティクス**において中心的な問題です。

Rust言語と [`chematic`](https://docs.rs/chematic/) クレートで実装された純粋なRust製エンジン。CLI・Pythonパッケージ・WASMモジュールを単一のRustコードベースから提供します。C/C++依存ゼロ。

---

## 特徴

| 特徴 | 詳細 |
|---|---|
| **Pure Rust** | C/C++依存ゼロ。`cargo build` のみで全プラットフォームに対応 |
| **A\* / AND-OR木探索** | MCTSより探索効率が高いことが実証されたRetro\*相当のアルゴリズム |
| **SA Scoreヒューリスティック** | `chematic::chem::sa_score` で合成困難度を考慮した探索優先度付け |
| **ビームサーチ** | `--beam-width N` でメモリ制約付き探索 |
| **グラフベース Ar-Ar 切断** | ブリッジボンドDFS検出でビアリール（Suzuki）切断を正確に処理 |
| **並列ルール適用** | `rayon` で SMIRKS ルールを並列評価（WASM では逐次にフォールバック） |
| **Pythonバインディング** | `import renkin; renkin.find_routes(...)` |
| **WASM対応** | 493 KB バンドル。2D構造式描画付きブラウザデモ |
| **約400件の市販原料** | エステル・アミン・ハロゲン化物・ヘテロ環・アミノ酸・スルホニルクロリド・ボロン酸など |
| **ベンチマークCLI** | `renkin-bench --input targets.smi` でJSON形式レポート |

---

## アーキテクチャ

```
目標 SMILES
     │
     ▼
┌─────────────────────────┐
│     chem_env.rs         │  ← chematic ラッパー
│  - SMILES パース        │     SMARTS VF2 による市販品判定
│  - SMIRKS 逆反応適用    │     フラグメント正規化
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
│      score.rs           │  ← ヒューリスティック評価関数
│  - SA Score             │     h = Σ(1 + 0.5·(sa−1)/9)
│  - 分子量ステップコスト  │     g = Σ(1 + total_mw/2000)
└────────────┬────────────┘
             │
             ▼
  JSON  ←  CLI / Python / WASM
```

---

## 技術スタック

- **言語**: Rust (Edition 2024) ※コアロジックはすべてRust
- **ケモインフォマティクス**: [`chematic`](https://crates.io/crates/chematic) v0.4.9+
  - `chematic-smiles` — SMILES パース・canonical SMILES 生成
  - `chematic-smarts` — VF2 サブ構造マッチング（市販品同一性判定）
  - `chematic-rxn` — SMIRKS 反応適用（`run_reactants`）
  - `chematic-chem` — SA Score・分子量・芳香族環数
- **探索アルゴリズム**: A\* 探索 + AND/OR木（Retro\* 相当）
- **並列化**: [`rayon`](https://crates.io/crates/rayon) — SMIRKS ルール並列適用
- **Python**: [`PyO3`](https://pyo3.rs) + [`maturin`](https://www.maturin.rs)
- **WASM**: [`wasm-bindgen`](https://rustwasm.github.io/wasm-bindgen/) + [`wasm-pack`](https://rustwasm.github.io/wasm-pack/)

---

## インストール

### ライブラリとして使う

```toml
# Cargo.toml
[dependencies]
renkin = "0.1"
```

### CLI（ソースからビルド）

```bash
git clone https://github.com/kent-tokyo/renkin
cd renkin
cargo build --release
```

### Python

```bash
pip install maturin
git clone https://github.com/kent-tokyo/renkin && cd renkin
python -m venv .venv && source .venv/bin/activate
maturin develop --features python
```

---

## クイックスタート

### CLI

```bash
# 実行例（アスピリン、深さ3）
./target/release/renkin --target "CC(=O)Oc1ccccc1C(=O)O" --depth 3

# ビームサーチ（上位50ノード）
./target/release/renkin --target "CC(=O)Oc1ccccc1C(=O)O" --depth 5 --beam-width 50
```

```
--target / -t      ターゲット分子 SMILES
--depth  / -d      最大逆合成深さ（デフォルト: 5）
--max-routes / -n  返すルート数上限（デフォルト: 5）
--beam-width / -w  ビーム幅、0 = 無制限 A*（デフォルト: 0）
--building-blocks  市販原料 .smi ファイルのパス
```

### Python

```python
import renkin, json

routes = json.loads(renkin.find_routes(
    "CC(=O)Oc1ccccc1C(=O)O",   # アスピリン
    depth=3,
    max_routes=5,
))
print(routes["routes_found"])
for r in routes["routes"]:
    print(r["depth"], [s["rule"] for s in r["steps"]])
```

### WASM

```bash
wasm-pack build --target web --no-default-features
# ブラウザデモ: python3 -m http.server 8080 → http://localhost:8080/demo/
```

```javascript
import init, { find_routes } from './pkg/renkin.js';
await init();

const result = JSON.parse(find_routes(
  "CC(=O)Oc1ccccc1C(=O)O",  // ターゲット SMILES
  3,   // depth
  5,   // max_routes
  0,   // beam_width (0 = 無制限)
));
console.log(result.routes_found);
```

### ベンチマーク

```bash
./scripts/run_benchmark.sh --input data/benchmark_targets.smi --depth 5
```

```json
{
  "total": 42, "solved": 37, "success_rate": 0.88,
  "avg_depth": 1.05, "avg_time_ms": 2.5
}
```

---

## 出力例（CLI）

```json
{
  "target": "CC(=O)Oc1ccccc1C(=O)O",
  "routes_found": 2,
  "routes": [
    {
      "steps": [
        {
          "rule": "ester_cleavage",
          "target": "CC(=O)Oc1ccccc1C(=O)O",
          "precursors": ["CC(=O)O", "Oc1ccccc1C(=O)O"]
        }
      ],
      "depth": 1
    }
  ]
}
```

**depth: 0** はターゲット自体が市販品（直接購入可能）を意味します。

---

## 逆合成ルール（全14件）

| ルール名 | 反応タイプ | 実装方式 |
|---|---|---|
| `ester_cleavage` | エステル → 酸 + アルコール | SMIRKS |
| `amide_cleavage` | アミド → 酸 + アミン | SMIRKS |
| `friedel_crafts_acylation_retro` | Ar-C(=O)R → Ar-H + 酸クロリド | SMIRKS |
| `aryl_carboxylation_retro` | Ar-COOH → Ar-H + CO₂ | SMIRKS |
| `aryl_amine_retro` | Ar-N → Ar-H + アミン | SMIRKS |
| `buchwald_hartwig_retro` | Ar-N → Ar-Br + アミン | SMIRKS |
| `aryl_ether_retro` | Ar-O → Ar-OH + フラグメント | SMIRKS |
| `suzuki_retro` | Ar-Ar → Ar-Br + Ar-H | グラフ（ブリッジボンドDFS） |
| `cc_single_cleavage` | C–C → 2フラグメント | SMIRKS |
| `wittig_retro` | C=C → C=O + C=O | SMIRKS |
| `reductive_amination_retro` | C–N → C=O + アミン | SMIRKS |
| `cn_aliphatic_cleavage` | C–N → 2フラグメント | SMIRKS |
| `co_aliphatic_cleavage` | C–O → 2フラグメント | SMIRKS |
| `alcohol_oxidation_retro` | C–OH → C=O | SMIRKS |

`suzuki_retro` はSMIRKSではなくグラフベースのブリッジボンド検出を使用し、対称ビアリール（biphenyl, 4-fluorobiphenyl等）を正確に処理します。

---

## プロジェクト構成

```
renkin/
├── Cargo.toml
├── src/
│   ├── lib.rs           # ライブラリクレート（DEFAULT_BUILDING_BLOCKS・再エクスポート）
│   ├── main.rs          # CLI バイナリ
│   ├── bin/
│   │   └── benchmark.rs # renkin-bench バイナリ
│   ├── chem_env.rs      # chematic ラッパー（パース・逆反応・市販品判定）
│   ├── score.rs         # SA Score ヒューリスティック + ステップコスト
│   ├── search.rs        # A* / AND-OR 木探索エンジン + ビーム枝刈り
│   ├── python.rs        # PyO3 バインディング（--features python）
│   └── wasm.rs          # wasm-bindgen バインディング（cfg = wasm32）
├── data/
│   ├── building_blocks.smi      # 市販原料（約400件）
│   └── benchmark_targets.smi   # ベンチマーク用42分子セット
├── demo/
│   └── index.html       # ブラウザWASMデモ（2D構造式描画付き）
└── scripts/
    └── run_benchmark.sh # ベンチマーク実行スクリプト
```

---

## ロードマップ

- [x] **Phase 1** — SMIRKS 逆反応ルール + フラグメント正規化
- [x] **Phase 2** — A\* / AND-OR 木探索、クローズドリスト、縮退ルートフィルタ
- [x] **Phase 3** — SA Score ヒューリスティック + ビームサーチ（`--beam-width`）
- [x] **Phase 4** — 並列ルール適用（rayon; WASM では逐次フォールバック）
- [x] **Phase 5** — Python バインディング（PyO3 + maturin）
- [x] **Phase 6** — WASM ビルド（493 KB、`pkg/` npm 配布可能）
- [x] **Phase 7** — ベンチマーク CLI（`renkin-bench`）
- [x] **Phase 8** — ユニットテスト21件、SMIRKSルール5→14件、市販原料~30→~400件
- [x] **Phase 9** — ブラウザWASMデモ（2D構造式描画）、ベンチマーク対象セット
- [x] **Phase 10** — グラフベースビアリール切断（suzuki_retro）、O(1) HashMapインデックス
- [ ] **Phase 11** — USPTO-50kデータセットでAiZynthFinder / Retro\*と正式比較
- [ ] **Phase 12** — PyPI / npm 公開

---

## 競合比較

| ツール | 言語 | アルゴリズム | WASM | ゼロ依存ビルド |
|---|---|---|---|---|
| **ASKCOS** | Python | MCTS / A\* | No | No（Docker、64 GB RAM） |
| **AiZynthFinder** | Python | MCTS 主体 | No | No（conda、モデルDL要） |
| **IBM RXN** | クローズド | Transformer | No | No（クラウドのみ） |
| **SYNTHIA** | クローズド | SMARTS + AND/OR | No | No（独自ライセンス） |
| **Retro\*** | Python | A\* + AND/OR | No | No（未メンテ） |
| **★ RENKIN** | **Rust** | **A\* + AND/OR** | **Yes** | **Yes（cargo build のみ）** |

既存のオープン CASP ツールはすべて Python 製。RENKINは**Rust製・WASM対応・依存関係ゼロ・A\*探索**という空白ニッチを埋めます。

---

## ライセンス

MIT
