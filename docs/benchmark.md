# Benchmark

## USPTO-50k Test Set

RENKIN を [USPTO-50k](https://huggingface.co/datasets/bisectgroup/USPTO_50K) テストセット全件（4,907 分子）で評価します。逆合成の標準ベンチマークデータセットです。

### Latest Results (v0.1.3) — depth=5, beam=100, 5,000 extracted templates

| Config | Solved | Success Rate | Avg Time | Hardware |
|--------|--------|-------------|----------|----------|
| depth=5, beam=100, 5,000 templates | **3,826 / 4,907** | **78.0%** | **2,775 ms/mol** | Apple M-series, 8 threads |

Building blocks: 480 hand-curated commercial reagents (default set).

### Progress History

| Version / Phase | Solved | Success Rate | Avg Time | Notes |
|-----------------|--------|-------------|----------|-------|
| v0.1.0 | 25 / 500 | 5.0% | 79 ms/mol | 20 rules, 480 BBs, depth=2, 500-mol sample |
| v0.1.1 (baseline) | 1,363 / 4,907 | 27.8% | — | default rules only, depth=3 |
| Phase A (500 templates, beam=100) | 2,315 / 4,907 | 47.2% | — | depth=5, +500 extracted templates |
| Phase A (5k templates, beam=100) | 3,540 / 4,907 | 72.1% | 1,742 ms/mol | depth=5, template frequency weighting |
| Phase A (5k templates, unlimited A\*) | 3,830 / 4,907 | 78.1% | 2,956 ms/mol | depth=5, beam=0 |
| Phase B (5k templates, beam=100, NN scorer) | 3,826 / 4,907 | 78.0% | 3,394 ms/mol | depth=5, ONNX neural scorer |
| **v0.1.3 (5k templates, beam=100)** | **3,826 / 4,907** | **78.0%** | **2,775 ms/mol** | depth=5, Pure Rust optimizations |

v0.1.3 では Pure Rust のままで Phase B (NN scorer 使用) と同等の精度を beam=100 で達成しました。

### Comparison with Other Systems

| System | Top-1 | Stock | Templates | Notes |
|--------|-------|-------|-----------|-------|
| **RENKIN v0.1.3** | **78.0%** | **480 BBs** | **5,000** | Pure Rust, no C++ dependencies |
| AiZynthFinder (Mol. Inf. 2020) | ~45% | eMolecules (~6M) | ~50,000 | Python, RDKit |
| Retro\* (ICML 2020) | ~40% | eMolecules (~6M) | ~50,000 | Python |
| LocalRetro (AAAI 2021) | ~65% | eMolecules (~6M) | template-free | GNN-based |
| GLN (NeurIPS 2020) | ~64% | eMolecules (~6M) | ~17,000 | GNN-based |

!!! note "条件の違い"
    RENKIN の 78.0% は **480 種類の市販試薬のみ**・**5,000 テンプレート**で達成しています。
    他システムは eMolecules 等の数百万化合物データベースと数万テンプレートを使用しており、
    RENKIN は不利な条件での評価です。

    RENKIN の強みは **Pure Rust・ゼロ C/C++ 依存・WASM/Python 対応** による移植性と組み込みやすさです。
    `cargo build` 一発でビルドでき、ブラウザ（WASM）・Python・CLI どこでも同一バイナリが動作します。

### What RENKIN solves well

標準的な結合切断に対して高い精度を示します：

- エステル → カルボン酸 + アルコール
- アミド → 酸 + アミン
- ビアリール → アリールハライド + ボロン酸（Suzuki）
- アリールアミン → アリールハライド + アミン（Buchwald-Hartwig）
- C–ハライド結合 → 脱ハロゲン化アレーン
- Boc / Cbz 保護基の脱保護

### Improving the success rate

より高い成功率を目指すには：

1. **在庫データベースの拡充** — eMolecules, ZINC, 社内在庫を `--building-blocks` で指定
2. **テンプレート数の増加** — USPTO 全データからより多くのテンプレートを抽出
3. **探索深度の増加** — `--depth 7` 等で多段階合成ルートをカバー

### Running the benchmark

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
