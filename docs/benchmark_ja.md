---
title: "RENKIN 過去のUSPTO-50kストレステスト（v0.15.5、凍結）"
description: "RENKIN v0.15.5、単一コミットに凍結されたUSPTO-50k route-to-stockストレステスト。詳細な手法と既知の限界を含む。他プランナーとの現在の比較データはOpen-Source Retrosynthesis Comparisonガイドを参照。"
---

# 過去のUSPTO-50kストレステスト（凍結、v0.15.5）

**このページ全体が凍結された過去の記録であり、現在進行形のベンチマークではありません。** 以下のすべての数値——「Corrected Baseline」セクションを含む——は、単一の特定コミット（`e20dc8c`、RENKIN v0.15.5、2026-07-22）に対して一度だけ計測されたものであり、それ以降再計測されていません。「Corrected」が指すのは、そのコミット時点でのルールセットであり、RENKINの現在の状態ではありません。このページのすべての数値は、ある1日にRENKINが行ったことのスナップショットとして扱ってください。

> **現在の・条件を揃えた比較データをお探しですか？** 代わりに
> [Open-Source Retrosynthesis Comparison](guides/open-source-retrosynthesis-comparison.md#500-target-results)
> ガイドを参照してください：500ターゲット、paired bootstrap、exact McNemar検定による、
> shared stockと各ツール自身のnative stockの両条件でのAiZynthFinderとの比較で、
> 常に最新の状態に保たれています。このページは最新の状態に保たれておらず、
> その目的には使用しないでください。

> ⚠️ **注記（2026-07-22）: このページに残る78.0%（単一パス）/95.9%（cascade）/81.8%（ChEMBL OOD）は無効化された過去の計測値であり、再計測されていません。** これらは、解決数を化学的に不正な経路や誤って肯定判定されたルートで水増ししていた4件の逆合成ルール・validatorバグを修正する前に計測されたものです（経緯は下記）。**「Corrected baseline」セクションのみが凍結時点のルールセットを反映しています**——このページの他の箇所は歴史的な連続性のために旧い無効化済みの数値をそのまま残しており、それぞれその旨を明記しています。マークの有無にかかわらず、このページのいかなる数値もRENKINの現在の性能として引用しないでください。
>
> **Corrected baseline — USPTO-50k Stage 1（単一パス）、コミット `e20dc8c`、2026-07-22。** Search-to-stock rate（`raw_solved_rate`）**20.09%**（986/4,907）→ atom-balance-filtered rate（`atom_balanced_solved_rate`）**15.41%**（756/4,907）→ current-validator-confirmed rate（`provenance_validated_solved_rate`）**0.88%**（43/4,907）。この3つは同一4,907件に対する*入れ子系列*（各段が前段のより厳格な部分集合）であり、独立した3つの計測値ではなく、いずれも単独では実験的に検証された合成成功率でも人間の化学者によるルート正確性評価でもありません。
>
> **0.88%が正確に何を表すか:** stockまで到達する完全な経路があり、かつ全stepがatom-balance checkを通過し、かつ全stepがそれぞれのoriginating ruleの現行validatorによって肯定的に確認された、その割合です。**これは実測の化学的正確性の値ではなく、真の正確性について数学的に証明された下限でもありません**——下限と言うにはvalidatorに偽陽性がないことの証明が必要ですが、それは示されていません（44件の`validated`ルートのうち1件はatom-balance checkを通らず、診断サンプルでも864 step中14 stepが`Valid`かつatom-imbalancedです）。同様に`Invalid`判定も誤りだと証明されたわけではありません：n=300診断サンプルで72.2%のstepが`Invalid`かつatom-balanced済みであり、そのうち未知の割合がvalidatorの偽陰性（正規化・互変異性・位置選択性の境界ケース）である可能性がある一方、実際のrule・route誤りも含まれ得ます。両者の比率は未計測です（`suzuki_retro`は0%invalid、`cn_aliphatic_cleavage`は97.6%invalidという広がりはさらなる調査に値しますが、いずれの原因にも未帰属です）。詳細な手法・設定ハッシュ・rule別内訳: [`tasks/phase31_final_remeasurement_run.md`](https://github.com/kent-tokyo/renkin/blob/master/tasks/phase31_final_remeasurement_run.md)。
>
> **修正の経緯:** `aryl_carboxylation_retro`のエステル誤発火を修正（PR #26）。原子を追跡なく消失させる同型バグを持つルール3件——`aryl_chloride_retro`・`aryl_iodide_retro`・`aryl_fluoride_snAr_retro`——を発見・削除（PR #31、「31.11」：いずれもハロゲン原子を追跡対象試薬なしに削除していました）。forward validatorが「実際にそのstepが使ったrule」ではなく「どれかのruleのSMIRKSが偶然targetを再現できればValid」としていた問題を、各stepの originating rule に拘束するよう修正（PR #33、「31.12」）。いずれもこの再計測前に、この順序で個別にCI確認の上でmasterへマージ済みです。**cascade（95.9%）・ChEMBL OOD（81.8%）は修正版ルールセットに対して未再計測**であり、無効化された過去の数値のまま残っています——各セクションの注記を参照してください。Phase 31のcorrected baseline確立・公開はこれで完了ですが、validatorの偽陰性と実際のrule誤りを切り分ける忠実性分析は、明示的な未解決の後続課題として残っています。

## USPTO-50k テストセット

USPTO-50kは主に**単一ステップ**逆合成のベンチマークとして使われています（単一ステップでの利用は下記の「比較: 単一ステップTop-1モデル」を参照）。このページでは、[USPTO-50k](https://huggingface.co/datasets/bisectgroup/USPTO_50K) から派生した4,907件の凍結ターゲットコーパスを、RENKINの多段階探索に対する**route-to-stockストレステスト**として転用しています——[PaRoutes](https://github.com/AstraZeneca/PaRoutes)（RENKINも`renkin-bench --input-format paroutes`で直接対応済み。[README](https://github.com/kent-tokyo/renkin#paroutes-compatibility)参照）のような、多段階の標準ベンチマークではありません。

このコーパスには、既知の・開示済みのprovenance gapもあります：`data/uspto50k_test.smi`のヘッダーは「5007 reactions」と記載していますが、実際のデータ行数は4,907件であり、このリポジトリにはこのファイルの由来となったHugging Face上の正確なrevisionを追跡する記録がありません。詳細な開示は
[Open-Source Retrosynthesis Comparisonガイドの「Known gaps」セクション](guides/open-source-retrosynthesis-comparison.md#known-gaps-disclosed-not-fixed-in-this-round)
を参照してください——同じ注意書きが2箇所で独立にずれていくのを避けるため、ここでは繰り返しません。

**「解決（solved）」の意味:** ターゲットは、すべてのリーフ前駆体がビルディングブロック集合に含まれる完全な逆合成経路が1つ以上見つかった場合に*solved*と判定されます（下記のcorrected-baseline実行では`data/building_blocks.smi`から読み込んだ402件のユニークな化合物——ファイルの生の行数と異なる理由は同セクション参照）。これはUSPTOデータセットのground-truth試薬との照合では**ありません**。

### Corrected Baseline（コミット `e20dc8c`、2026-07-22） — depth=5, beam=100, 5,000 extracted templates, 28 hand-crafted rules

| Public label | 内部指標 | 値 | 分母 |
|------|------|-----|------|
| Search-to-stock rate | `raw_solved_rate` | **20.09%**（986/4,907） | 全4,907件 |
| Atom-balance-filtered rate | `atom_balanced_solved_rate` | **15.41%**（756/4,907） | 全4,907件——search-to-stockの部分集合 |
| Current-validator-confirmed rate | `provenance_validated_solved_rate` | **0.88%**（43/4,907） | 全4,907件——atom-balance-filteredの部分集合。この値が何であり何でないかは上記注記を参照 |
| depth=0 直接stock一致 | — | 0.04%（2/4,907） | 全4,907件 |
| レイテンシ（全ターゲット） | — | p50 7.3s / p95 28.2s / p99 51.2s | 未解決ターゲット（探索予算まで実行）を含む |
| レイテンシ（解決済のみ） | — | p50 1.0s / p95 9.4s / p99 15.6s | |

この3指標は同一4,907件に対する入れ子系列であり独立した数値ではなく、いずれも単独では化学的正確性を実測・保証するものではありません——単独で引用する前に上記の注記を参照してください。

ビルディングブロック: `ChemEnv::load("data/building_blocks.smi")`（`ChemEnv::bb_count()`）が実際にロードした**402**件のユニークな化合物——parse・canonical化・重複除去後の数であり、ファイルの生の行数ではありません（非コメント行449 → parse失敗3 → canonical化後の重複44 → ユニーク402）。詳細な設定・ハッシュ・rule別内訳は `tasks/phase31_final_remeasurement_run.md` を参照してください。

### 過去の結果（v0.15.5、修正前） — depth=5, beam=100, 約5,000 extracted templates

| 設定 | 解決数 | 成功率 | 平均時間 | 実行環境 |
|------|--------|--------|----------|----------|
| depth=5, beam=100, 約5,000テンプレート + Phase A | **3,826 / 4,907** | **78.0%** | **≈2,800 ms/mol** | Apple M-series, 8スレッド |

*状態: 無効化された過去の計測値、31.11/31.12修正前——上記の注記を参照。継続性のためだけに保持。*

### 進捗の推移（Table A — RENKIN内部）

| フェーズ | 解決数 | 成功率 | 備考 |
|-------|--------|------|-------|
| ルール31件のみ、depth=3 | 366 / 4,907 | 7.5% | hand-crafted ruleのみ |
| + 抽出テンプレート191件、depth=3 | 1,363 / 4,907 | 27.8% | rdchiral top-300 |
| + depth=5 | 1,909 / 4,907 | 38.9% | depth増加 |
| + top-500テンプレート、depth=5 | 2,315 / 4,907 | 47.2% | 合計314ルール |
| + beam=100 | 2,688 / 4,907 | 54.8% | ビームサーチ |
| + Phase A頻度重み付け | 3,540 / 4,907 | 72.1% | 高頻度テンプレートへのstep_costボーナス |
| **+ 約5,000テンプレート（v0.15.5）** | **3,826 / 4,907** | **78.0%** | 修正前計測値、無効化済み |
| **Cascade: Stage 2（depth=7, beam=300, 未解決分のみ）** | **4,705 / 4,907** | **95.9%** | 2026-06-29 ✅（修正前計測値、無効化済み） |

*状態: 無効化された過去の計測値——上記の注記を参照。*

### 比較: 単一ステップTop-1モデル（異なる指標）

> **⚠️ 異なる指標です。** これらは単一ステップのtop-1予測精度（モデルのtop-1予測が既知の反応と一致するか）を測定するものであり、多段階プランニングの成功率では**ありません**。上記のRENKINの多段階成功率との直接比較は妥当ではありません。

| システム | 単一ステップTop-1 | 出典 |
|--------|------------------|--------|
| LocalRetro | 53.4% | Chen et al., ACS Cent. Sci. 2021 |

!!! note "RENKINの設定"
    上記のRENKINの探索到達率20.09%は **402種類の市販試薬のみ**・**約5,000テンプレート**で達成しています。RENKIN の強みは**移植性**です：Pure Rust、ゼロ C/C++ 依存、単一バイナリで WASM + Python + CLI に対応します。

### RENKIN が得意とする反応

> ⚠️ **実測の精度に基づく主張ではありません。** 以下のリストは、RENKINがhand-crafted
> またはgraph-baseの明示的なルールを持っている変換ファミリーを示したものであり、
> クラス別に再計測された精度の数値ではありません（修正済みルールセットに対する
> そのような数値は存在しません。このセクションが以前暗黙に依拠していた
> 78.0%/95.9%という過去の数値は無効化されています——本ページ冒頭の注記を参照）。

RENKIN は以下の変換ファミリーに対して明示的なルールを持っています：

- エステル → カルボン酸 + アルコール
- アミド → 酸 + アミン（グラフベース切断）
- ビアリール → アリールハライド + ボロン酸（Suzuki、グラフベース）
- アリールアミン → アリールハライド + アミン（Buchwald-Hartwig）
- Boc / Cbz 保護基の脱保護（グラフベース）
- ジアリールスルホン → アリールスルホニルクロリド + アレーン（グラフベース）
- スルホンアミド → スルホニルクロリド + アミン

### ドメイン外（OOD）評価

> ⚠️ **修正版ルールセットに対して未再計測です（31.11/31.12以前、無効化済み）。** 以下の2行はいずれも過去の参考値として扱ってください。

RENKIN の精度が USPTO-50k ドメインに固有かどうかを確認するため、ChEMBL の **FDA承認薬500件**（Phase 4、MW 150–700、塩を除外、2026-06-25）で評価しました。

| データセット | 解決数 | 成功率 | 備考 |
|------------|--------|--------|------|
| USPTO-50k テストセット（in-domain） | 3,826 / 4,907 | **78.0%**（修正前） | テンプレートはUSPTO訓練セットから抽出 |
| **ChEMBL 承認薬（OOD）** | **409 / 500** | **81.8%**（修正前） | 実際のFDA承認医薬品 |

承認薬での+3.8ポイントの差は、ルールセットが医薬品合成で一般的な変換を広くカバーしているという仮説と整合します。ただし、この結果は慎重に解釈すべきです：両データセットともsmall-molecule有機化学であり、OODギャップとしての振れ幅は限定的です。両データセットで未解決分子は共通のプロファイルを示します：窒素の多い複素環（+17ポイント）とフッ素化合物（+11ポイント）。

### 失敗要因の分類（2026-06-29、500分子サンプル）

> ⚠️ **修正版ルールセットに対して未再計測です（31.11/31.12以前、無効化済み）。** 手法の参考としてのみ保持しています。

`renkin-bench --failure-taxonomy` は未解決ターゲットを原因別に分類します：

| 原因 | 件数 | 未解決分に占める割合 | 説明 |
|-------|-------|--------------|------|
| beam_limit_hit | 111 / 112 | 99.1% | beamが有望なノードを枝刈りした |
| max_depth_reached | 111 / 112 | 99.1% | depth=5を超えるルートが必要だった |
| stock_near_miss | 111 / 112 | 99.1% | frontierにBBはあったが完全な経路がなかった |
| no_template_match | 1 / 112 | 0.9% | マッチしたテンプレートが3件未満だった |

**主な知見:** テンプレートやビルディングブロックのカバレッジはボトルネックではありません。未解決ターゲットのほぼすべてが探索予算（beam/depth）の上限に達しています。Cascade search（Stage 2: 未解決分のみdepth=7, beam=300で再実行）は、以前未解決だった1,081件中879件（81.3%）を解決し、全体の成功率を78.0%から**95.9%**へ引き上げました。

### 成功率をさらに高めるには

1. **Cascade search** — 未解決ターゲットをより高いbeam/depthで再実行（`--depth 7 --beam-width 300`）。失敗要因分析では、これが主要なレバーであることが示されています。
2. **ビルディングブロックデータベースの拡充** — eMolecules、ZINC、社内在庫を `--building-blocks` で指定
3. **テンプレートの追加** — USPTO全訓練セットからより多くのテンプレートを抽出（`--templates data/templates_extracted_5000.smi`）

### ベンチマークの実行方法

```bash
# ビルド
cargo build --release

# 全件ベンチマーク — 50チャンク × 100分子、中断・再開可能
bash scripts/run_benchmark_chunks.sh \
    data/uspto50k_test.smi \
    data/templates_extracted_5000.smi \
    data/bench_chunks \
    5 100

# 未解決分の失敗要因分析
./target/release/renkin-bench \
    --input data/uspto50k_test.smi \
    --depth 5 --beam-width 100 \
    --templates data/templates_extracted_5000.smi \
    --failure-taxonomy \
    > bench_result.json

# チャンクの集計
python3 -c "
import json, glob
files = sorted(glob.glob('data/bench_chunks/chunk_*.json'))
total = solved = 0
for f in files:
    d = json.load(open(f))
    total += d['total']; solved += d['solved']
print(f'{solved}/{total} = {solved/total:.1%}')
"
```
