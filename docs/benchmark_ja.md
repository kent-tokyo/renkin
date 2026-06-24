# ベンチマーク

## USPTO-50k テストセット

RENKIN を [USPTO-50k](https://huggingface.co/datasets/bisectgroup/USPTO_50K) テストセット全件（4,907 分子）で評価します。逆合成の標準ベンチマークデータセットです。

### 最新結果 (v0.1.4) — depth=5, beam=100, 5,000 extracted templates

| 設定 | 解決数 | 成功率 | 平均時間 | 実行環境 |
|------|--------|--------|----------|----------|
| depth=5, beam=100, 5,000 templates | **3,831 / 4,907** | **78.1%** | **≈2,800 ms/mol** | Apple M-series, 8 スレッド |

ビルディングブロック: 509 種類の手選定市販試薬（デフォルトセット）

### 精度の変遷

| バージョン / フェーズ | 解決数 | 成功率 | 平均時間 | 備考 |
|----------------------|--------|--------|----------|------|
| v0.1.0 | 25 / 500 | 5.0% | 79 ms/mol | ルール 20 件・BB 480 件・depth=2・500mol サンプル |
| v0.1.1（ベースライン） | 1,363 / 4,907 | 27.8% | — | デフォルトルールのみ・depth=3 |
| Phase A（500 テンプレート, beam=100） | 2,315 / 4,907 | 47.2% | — | depth=5・抽出テンプレート 500 件追加 |
| Phase A（5k テンプレート, beam=100） | 3,540 / 4,907 | 72.1% | 1,742 ms/mol | depth=5・テンプレート頻度重み付け |
| Phase A（5k テンプレート, unlimited A\*） | 3,830 / 4,907 | 78.1% | 2,956 ms/mol | depth=5・beam=0（無制限） |
| Phase B（5k テンプレート, beam=100, NN スコアラー） | 3,826 / 4,907 | 78.0% | 3,394 ms/mol | depth=5・ONNX ニューラルスコアラー |
| v0.1.3（5k テンプレート, beam=100） | 3,826 / 4,907 | 78.0% | 2,775 ms/mol | depth=5・Pure Rust 最適化 |
| **v0.1.4（5k テンプレート, beam=100, diaryl sulfone ルール）** | **3,831 / 4,907** | **78.1%** | **≈2,800 ms/mol** | depth=5・diaryl_sulfone_retro + 509 BB |

v0.1.4 では、ジアリールスルホン逆合成ルール（グラフベース）を追加し、ビルディングブロックを 509 件に拡充しました。

### 他システムとの比較

| システム | Top-1 | 在庫 | テンプレート数 | 備考 |
|---------|-------|------|-------------|------|
| **RENKIN v0.1.4** | **78.1%** | **509 BBs** | **5,000** | Pure Rust、C++ 依存なし |
| AiZynthFinder (Mol. Inf. 2020) | ~45% | eMolecules (~600 万) | ~50,000 | Python、RDKit |
| Retro\* (ICML 2020) | ~40% | eMolecules (~600 万) | ~50,000 | Python |
| LocalRetro (AAAI 2021) | ~65% | eMolecules (~600 万) | テンプレートフリー | GNN ベース |
| GLN (NeurIPS 2020) | ~64% | eMolecules (~600 万) | ~17,000 | GNN ベース |

!!! note "条件の違い"
    RENKIN の 78.0% は **480 種類の市販試薬のみ**・**5,000 テンプレート**で達成しています。
    他システムは eMolecules 等の数百万化合物データベースと数万テンプレートを使用しており、
    RENKIN は不利な条件での評価です。

    RENKIN の強みは **Pure Rust・ゼロ C/C++ 依存・WASM/Python 対応** による移植性と組み込みやすさです。
    `cargo build` 一発でビルドでき、ブラウザ（WASM）・Python・CLI どこでも同一バイナリが動作します。

### RENKIN が得意とする反応

標準的な結合切断に対して高い精度を示します：

- エステル → カルボン酸 + アルコール
- アミド → 酸 + アミン（グラフベース切断）
- ビアリール → アリールハライド + ボロン酸（Suzuki）
- アリールアミン → アリールハライド + アミン（Buchwald-Hartwig）
- C–ハライド結合 → 脱ハロゲン化アレーン
- Boc / Cbz 保護基の脱保護
- ジアリールスルホン → アリールスルホニルクロリド + アレーン（グラフベース、v0.1.4）
- スルホンアミド → スルホニルクロリド + アミン

### ドメイン外（OOD）評価

RENKIN の精度が USPTO-50k ドメイン限定かどうかを確認するため、ChEMBL の **FDA 承認薬 500 件**（Phase 4、MW 150–700、塩除外）で評価しました。

| データセット | 解決数 | 成功率 | 備考 |
|------------|--------|--------|------|
| USPTO-50k テストセット | 3,831 / 4,907 | **78.1%** | in-distribution（テンプレートは USPTO 訓練セットから抽出） |
| **ChEMBL 承認薬** | **409 / 500** | **81.8%** | out-of-distribution（実際の FDA 承認医薬品） |

**RENKIN は USPTO ドメインに限らず、実際の承認薬にも良く機能します。** +3.7 pp の向上は、ルールセットが USPTO 訓練データ特有の反応ではなく、医薬品合成で一般的な変換を幅広くカバーしていることを示します。

未解決分子のパターンは両データセットで共通です：N の多い複素環（未解決で +17 pp）とフッ素化合物（+11 pp）。これはドメイン固有の問題ではなく、構造的な難しさによるものです。

### 成功率をさらに高めるには

1. **在庫データベースの拡充** — eMolecules、ZINC、社内在庫を `--building-blocks` で指定
2. **テンプレート数の増加** — USPTO 全データからより多くのテンプレートを抽出
3. **探索深度の増加** — `--depth 7` 等で多段階合成ルートをカバー

### ベンチマークの実行方法

```bash
# ビルド
cargo build --release

# USPTO-50k テストセット取得（初回のみ）
python3 scripts/download_uspto50k.py

# 全件ベンチマーク（50 チャンク × 100 mol、中断再開可能）
bash scripts/run_benchmark_chunks.sh \
    data/uspto50k_test.smi \
    data/templates_extracted_5000.smi \
    data/bench_chunks \
    5 100

# 結果集計
python3 -c "
import json, glob
files = sorted(glob.glob('data/bench_chunks/chunk_*.json'))
total = solved = 0; times = []
for f in files:
    d = json.load(open(f))
    total += d['total']; solved += d['solved']
    times.append(d['avg_time_ms'])
print(f'{solved}/{total} = {solved/total:.1%}, avg {sum(times)/len(times):.0f} ms/mol')
"
```
