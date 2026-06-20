# RENKIN

RENKIN = **R**etrosynthetic **E**xploration **N**etwork for **K**nowledge-**I**nformed **N**avigation

> 錬金（れんきん）― 錬金術のように、目標分子を安価な原料へと逆変換する。  
> **超快適・軽量・高速**な逆合成エンジン。

[English README](./README.md)

---

## RENKINとは

RENKINは、目標分子（ゴール）から逆算して市販の安価な原料へと至る最適な化学反応経路を自動発見する**逆合成（Retrosynthesis）エンジン**です。

Rust言語と [`chematic`](https://docs.rs/chematic/) クレートで実装された純粋なRust製エンジン。CLI・Pythonパッケージ・WASMモジュールを単一のRustコードベースから提供します。

---

## 特徴

| 特徴 | 詳細 |
|---|---|
| **Pure Rust** | C/C++依存ゼロ。`cargo build` のみで全プラットフォームに対応 |
| **A\* / AND-OR木探索** | MCTSより探索効率が高いことが実証されたRetro\*相当のアルゴリズム |
| **SA Scoreヒューリスティック** | `chematic::chem::sa_score` で合成困難度を考慮した探索優先度付け |
| **ビームサーチ** | `--beam-width N` でメモリ制約付き探索 |
| **並列ルール適用** | `rayon` で SMIRKS ルールを並列評価（WASM では逐次にフォールバック） |
| **Pythonバインディング** | `import renkin; renkin.find_routes(...)` |
| **WASM対応** | 493 KB バンドル。サーバー不要でブラウザ上で動作 |
| **ベンチマークCLI** | `renkin-bench --input targets.smi` でJSON形式の成功率・時間レポート |

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
│  - 市販品チェック       │
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

- **言語**: Rust (Edition 2024) ※メイン言語
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

## クイックスタート

### CLI

```bash
# ビルド
cargo build --release

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

```bash
# インストール（Python ≥ 3.8 と maturin が必要）
python -m venv .venv && source .venv/bin/activate
maturin develop --features python
```

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
# 出力: pkg/  (npmパッケージ)
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
# 入力: 1行1SMILES、スペース区切りで名前を指定可能
cargo run --bin renkin-bench -- --input targets.smi --depth 3
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
          "target": "c1cccc(c1OC(=O)C)C(=O)O",
          "precursors": ["c1c(C(=O)O)cccc1", "OC(C)=O", "C", "Oc1c(cccc1)C(O)=O"]
        }
      ],
      "depth": 1
    }
  ]
}
```

**depth: 0** はターゲット自体が市販品（直接購入可能）を意味します。

---

## 逆合成ルール

| ルール名 | 反応タイプ | SMIRKS |
|---|---|---|
| `ester_cleavage` | エステル → 酸 + アルコール | `[C:1](=[O:2])[O:3]>>[C:1](=[O:2])O.[O:3]` |
| `amide_cleavage` | アミド → 酸 + アミン | `[C:1](=[O:2])[N:3]>>[C:1](=[O:2])O.[N:3]` |
| `aryl_carboxylation_retro` | Ar-COOH → Ar + CO₂ | `[c:1][C:2](=O)O>>[c:1].[C:2](=O)O` |
| `aryl_amine_retro` | Ar-NH₂ → Ar + NH₃ | `[c:1][N:2]>>[c:1].[N:2]` |
| `cc_single_cleavage` | C–C 切断 | `[C:1][C:2]>>[C:1].[C:2]` |

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
│   └── building_blocks.smi  # 市販原料（約30件）
└── pkg/                 # WASM npm パッケージ（wasm-pack 生成物）
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
- [ ] **Phase 7+** — USPTO-50k データセットで AiZynthFinder / Retro\* と正式比較

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
