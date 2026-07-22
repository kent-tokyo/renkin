# ベンチマーク

> ⚠️ **注記（2026-07-22）: このページの78.0%/95.9%/81.8%(ChEMBL)は無効化された過去の計測値であり、再計測されていません。** 「Corrected baseline」セクションのみが現行ルールセットの数値です。
>
> **Corrected baseline — USPTO-50k Stage 1（単一パス）、コミット `e20dc8c`、2026-07-22。** Search-to-stock rate（`raw_solved_rate`）**20.09%**（986/4,907）→ Atom-balance-filtered rate（`atom_balanced_solved_rate`）**15.41%**（756/4,907）→ Current-validator-confirmed rate（`provenance_validated_solved_rate`）**0.88%**（43/4,907）。この3つは同一4,907件に対する入れ子系列（各段はより厳格な部分集合）であり、独立した3つの計測値ではなく、いずれも実験的に検証された合成成功率でも人間の化学者によるルート正確性評価でもない。
>
> **0.88%が正確に何を表すか:** stockまで到達する完全な経路があり、かつ全stepがatom-balance checkを通過し、かつ全stepがそれぞれのoriginating ruleの現行validatorで肯定的に確認された、その割合。**これは実測の化学的正確性の値ではなく、真の正確性について数学的に証明された下限でもない**——下限と言うにはvalidatorに偽陽性がないことの証明が必要だが、それは示されていない（44件のValidatedのうち1件はatom-balance checkを通らず、診断サンプルでも864 step中14 stepがValidかつatom-imbalanced）。同様に`Invalid`判定も誤りだと証明されたわけではない：n=300診断サンプルで72.2%のstepがInvalidだがatom-balanced済みであり、そのうち未知の割合がvalidator偽陰性（正規化・互変異性・位置選択性の境界ケース）である可能性がある一方、実際のrule・route誤りも含まれ得る。両者の比率は未計測である（`suzuki_retro`は0%invalid、`cn_aliphatic_cleavage`は97.6%invalidという広がりはさらなる調査に値するが、いずれの原因にも未帰属）。詳細な手法・ハッシュ・rule別内訳: [`tasks/phase31_final_remeasurement_run.md`](https://github.com/kent-tokyo/renkin/blob/master/tasks/phase31_final_remeasurement_run.md)。
>
> **修正の経緯:** `aryl_carboxylation_retro`のエステル誤発火を修正（PR #26）。原子を追跡なしに消失させる同型バグを持つルール3件——`aryl_chloride_retro`・`aryl_iodide_retro`・`aryl_fluoride_snAr_retro`——を発見・削除（PR #31、「31.11」）。forward validatorが「実際に使ったrule」ではなく「どれかのruleが偶然再現できればValid」としていた問題を、各stepの originating rule に拘束するよう修正（PR #33、「31.12」）。いずれもこの再計測前に個別CI確認の上でmasterへマージ済み。**cascade（95.9%）・ChEMBL OOD（81.8%）は修正版ルールセットに対して未再計測**——各セクションの注記を参照。Phase 31のcorrected baseline確立・公開はこれで完了。validatorの偽陰性と実際のrule誤りの分離調査は後続課題として残る。

## USPTO-50k テストセット

RENKIN を [USPTO-50k](https://huggingface.co/datasets/bisectgroup/USPTO_50K) テストセット全件（4,907 分子）で評価します。逆合成の標準ベンチマークデータセットです。

### Corrected Baseline（コミット `e20dc8c`、2026-07-22） — depth=5, beam=100, 5,000 extracted templates, 28 handcrafted rules

| Public label | 指標 | 値 | 分母 |
|------|------|-----|------|
| Search-to-stock rate | `raw_solved_rate` | **20.09%**（986/4,907） | 全4,907件 |
| Atom-balance-filtered rate | `atom_balanced_solved_rate` | **15.41%**（756/4,907） | 全4,907件——search-to-stockの部分集合 |
| Current-validator-confirmed rate | `provenance_validated_solved_rate` | **0.88%**（43/4,907） | 全4,907件——atom-balance-filteredの部分集合。詳細は上記注記参照 |
| depth=0 直接stock一致 | — | 0.04%（2/4,907） | 全4,907件 |
| レイテンシ（全ターゲット） | — | p50 7.3s / p95 28.2s / p99 51.2s | 未解決ターゲット（探索予算まで実行）を含む |
| レイテンシ（解決済のみ） | — | p50 1.0s / p95 9.4s / p99 15.6s | |

この3指標は同一4,907件に対する入れ子系列であり独立した数値ではなく、いずれも単独では化学的正確性を実測・保証するものではない——引用前に上記注記を参照。

ビルディングブロック: `ChemEnv::load("data/building_blocks.smi")`が実際にロードした**402**件のユニークな化合物（`ChemEnv::bb_count()`）——parse・canonical化・重複除去後の数であり、ファイルの生の行数ではない（非コメント行449 → parse失敗3 → canonical化後の重複44 → ユニーク402）。詳細な設定・ハッシュ・rule別内訳は `tasks/phase31_final_remeasurement_run.md` を参照。

### 過去の結果 (v0.1.8、修正前) — depth=5, beam=100, 5,000 extracted templates

| 設定 | 解決数 | 成功率 | 平均時間 | 実行環境 |
|------|--------|--------|----------|----------|
| depth=5, beam=100, 5,000 templates | **3,831 / 4,907** | **78.1%** | **≈2,800 ms/mol** | Apple M-series, 8 スレッド |

*状態: 無効化された過去の計測値、31.11/31.12修正前——上記の注記を参照。参考として保持。*

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
| **v0.1.8（5k テンプレート, beam=100, diaryl sulfone ルール）** | **3,831 / 4,907** | **78.1%** | **≈2,800 ms/mol** | depth=5・diaryl_sulfone_retro + 509 BB |

*状態: 無効化された過去の計測値——上記の注記を参照。*

v0.1.8 では、ジアリールスルホン逆合成ルール（グラフベース）を追加し、ビルディングブロックを 509 件（当時のドキュメント記載値。`bb_count()`での再検証はしていないlegacy documentation value）に拡充したと記録されています。

### 他システムとの比較

| システム | Top-1 | 在庫 | テンプレート数 | 備考 |
|---------|-------|------|-------------|------|
| **RENKIN（corrected, raw_solved_rate）** | **20.09%** | **402 BBs** | **5,000** | Pure Rust、C++ 依存なし、2026-07-22 |
| AiZynthFinder (Mol. Inf. 2020) | ~45% | eMolecules (~600 万) | ~50,000 | Python、RDKit |
| Retro\* (ICML 2020) | ~40% | eMolecules (~600 万) | ~50,000 | Python |
| LocalRetro (AAAI 2021) | ~65% | eMolecules (~600 万) | テンプレートフリー | GNN ベース |
| GLN (NeurIPS 2020) | ~64% | eMolecules (~600 万) | ~17,000 | GNN ベース |

RENKIN行は`raw_solved_rate`（stockへの経路が1件以上見つかった率）を使用——他システムが公表しているroute-finding成功率に最も近いRENKIN側の指標です。ただし在庫規模・テンプレート集合・ターゲット集合・探索budget・ルート品質検査がシステムごとに異なるため、直接比較はできず、この表はRENKINが他システムより優れている（あるいは劣っている）ことを示すものではありません。RENKIN はこれに加えてより厳格な入れ子指標（`atom_balanced_solved_rate` 15.41%、`provenance_validated_solved_rate` 0.88%——上記の注記参照）も報告していますが、他システムの論文には直接対応する数値がありません。

!!! note "条件の違い"
    RENKIN の 20.09% は **402 種類の市販試薬のみ**・**5,000 テンプレート**で達成しています。
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
- Boc / Cbz 保護基の脱保護
- ジアリールスルホン → アリールスルホニルクロリド + アレーン（グラフベース、v0.1.8）
- スルホンアミド → スルホニルクロリド + アミン

### ドメイン外（OOD）評価

> ⚠️ **修正版ルールセットに対して未再計測（31.11/31.12以前、無効化済み）。** 以下の2行はいずれも過去の参考値です。

RENKIN の精度が USPTO-50k ドメイン限定かどうかを確認するため、ChEMBL の **FDA 承認薬 500 件**（Phase 4、MW 150–700、塩除外）で評価しました。

| データセット | 解決数 | 成功率 | 備考 |
|------------|--------|--------|------|
| USPTO-50k テストセット | 3,831 / 4,907 | **78.1%**（修正前） | in-distribution（テンプレートは USPTO 訓練セットから抽出） |
| **ChEMBL 承認薬** | **409 / 500** | **81.8%**（修正前） | out-of-distribution（実際の FDA 承認医薬品） |

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
