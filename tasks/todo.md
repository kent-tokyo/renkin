# RENKIN - Todo

## Phase 30: quietset × RENKIN 統合

quietset（`cargo install quietset-cli`）を使い、複数設定を跨いで安定したルート・ベンチターゲットだけを残す。
「RENKIN が候補を出す、quietset が安定した確信だけ残す」役割分担。

- [x] **30.1** Phase 1: `renkin-bench --quietset-out <file>` フラグ実装（2026-06-28）
  - `--quietset-out <path>` — 追記モードで quietset 互換 JSONL を書き出し
  - `--evaluator-id <id>` — evaluator 名指定（省略時 `renkin-d{depth}-b{beam}` を自動生成）
  - フィールド: `sample_id=name, label=solved/unsolved, score=best_success_prob, budget=beam_width, seed=1`
  - 変更: `src/bin/benchmark.rs` のみ、新規依存ゼロ
- [x] **30.2** Phase 2: multi-config 安定性ワークフロー（shell script）（2026-06-28）
  - `scripts/bench_stability.sh` — 複数 beam でベンチ → observations.jsonl 蓄積 → `quietset score/filter` まで自動実行
  - オプション: `--beams 50,100,200` / `--depth` / `--templates` / `--building-blocks` / `--out-dir` / `--min-observations`
  - quietset 未インストール時はインストール方法と手動コマンドを表示して graceful exit
- [ ] **30.3** Phase 3: MCP tool 化（Phase 2 で価値確認後）
  - `find_stable_routes` — 複数条件でルートを生成し安定ルートだけ返す
  - `explain_route_stability` — ルートの安定性スコアの内訳を説明

---

## Phase 29: 機能ロードマップ（次フェーズ候補）

優先順位は「使える体験に変換」を軸に設定。

- [x] **29.1** `renkin-doctor` — 環境診断コマンド（PR #8 マージ済み）
- [ ] **29.2** `docs/site-sync` — docs サイトの残り古い記述を整理（api/rust.md 等）
- [ ] **29.3** `feat/playground-route-cards` — confidence/cost/atom_economy カード表示
  - Copy CLI / Copy Python / Copy JSON / Copy Mermaid ボタン
  - Constraint UI（avoid/require/depth/beam）、プリセット分子
- [x] **29.4** `renkin-bench compare` 実装（2026-06-28）
  - `renkin-bench compare <baseline.json> <current.json>` — 成功率 delta・新規解決・退行を表示
  - 残: `--format markdown` / `benchmark.lock`（YAGNI、必要になったら追加）
- [ ] **29.5** `feat/stock-import` — stock DB 管理 CLI
  - `renkin stock import building_blocks.smi`、`renkin stock import-prices prices.csv`
  - `renkin stock stats / validate / coverage data/uspto50k_test.smi`
- [x] **29.6** `feat/mcp-tools` — MCP `diagnose_failure` ツール追加（2026-06-28）
  - `diagnose_failure` — ルートが見つからなかった理由を SearchStats から診断し具体的な提案を返す
  - 残: `compare_routes`、`estimate_route_cost`（必要になったら追加）

---

## Phase 28: OSS 信頼性強化 ✅ 完了（2026-06-27）

release CI の修正、セキュリティ整備、バージョン管理の自動化。

- [x] **28.1** README バッジ 3 段構成に整理（Status / Distribution / Features）
  - docs.rs / Python versions / Security Audit / PyO3 / MCP / benchmark バッジ追加
  - Colab バッジを Quick Start セクションへ移動
- [x] **28.2** `Why RENKIN?` セクション追加（README / README_ja）
- [x] **28.3** SECURITY.md 追加 → GitHub Security policy: Enabled
- [x] **28.4** `.github/dependabot.yml` — Cargo / npm / pip / GitHub Actions 週次更新
- [x] **28.5** `.github/workflows/security-audit.yml` — rustsec/audit-check push/PR/週次
- [x] **28.6** CI 全ジョブに `permissions:` 追加（CodeQL アラート 7 件解消）
- [x] **28.7** `release.yml` smoke test 修正: `renkin.version()` → `renkin.__version__`
- [x] **28.8** `release.yml` PyPI 伝播リトライ（`sleep 60` 一発 → 5 回 × 60s）
- [x] **28.9** `ci.yml` に `python-smoke` ジョブ追加（Python API を PR ごとに検証、事前ゲート）
- [x] **28.10** `ci.yml` に `version-check` ジョブ追加（docs/installation・README Citation が Cargo.toml と一致するか検証）
- [x] **28.11** docs バージョン不整合を修正（`renkin = "0.1"` → `"0.15"` 等、5 箇所）
- [x] **28.12** `.github/pull_request_template.md` 追加（release 時の version sync チェックリスト）
- [x] **28.13** `CONTRIBUTING.md` にブランチ命名規則追加（`feat/*` / `fix/*` / `docs/*` / `release/*`）
- [x] **28.14** master ブランチ保護設定（GitHub API 経由）
  - Required checks: Test / Lint / Version sync / Python smoke
  - strict: true（最新 master と同期必須）、force push 禁止、branch delete 禁止
- [x] **28.15** v0.15.4 リリース（smoke test 修正等を含む初の "clean" リリース）
- [x] **28.16** `GitHub star` CTA を README / README_ja 末尾に追加

---

## Phase 27: 制約付き探索 + 探索トレース ✅ 完了（2026-06-25）

- [x] **27.1** `--avoid-elements / -e` — forbidden element bitmask post-filter
  - `SearchConfig.forbidden_elements: u64` 追加
  - 葉 BB に指定元素が含まれるルートを除外
  - Python 対応: `avoid_elements=""` 引数
- [x] **27.2** `--require-elements / -r` — required element presence filter
  - `SearchConfig.required_element_present: u64` 追加
  - 葉 BB の和集合が指定元素を全てカバーするルートのみ採用
  - 組み合わせ例: `--require-elements B --avoid-elements Br,I` → biphenyl 5→1 ルート
- [x] **27.3** `--verbose / -v` — search trace to stderr
  - `nodes_popped`, `nodes_expanded`, `routes_found`, `elapsed` を出力
  - stdout（JSON/tree/mermaid）は無影響
- [x] **27.4** `chem_env::elem_symbols_to_mask()` helper 追加
- [x] **27.5** README に Constraint-based Search セクション追加（before/after 実出力つき）
- [x] **27.6** バグ修正: display.rs dead code / train_template_scorer.py returncode チェック
- [x] **27.7** `.gitignore` に `data/*.onnx` 追加

---

## Phase 26: ルート可視化 ✅ 完了（2026-06-25）

- [x] `--format tree` — ASCII tree（ルール名・BB マーカー・スコア・depth）
- [x] `--format mermaid` — GitHub/Notion 対応 Mermaid flowchart
- [x] `Route.score: f64` — JSON 出力に A* コスト追加
- [x] `src/display.rs` 新モジュール

---

## Phase 1–12: 完了済み ✅

- [x] 1.1 SMILES パース（chematic）
- [x] 1.2 SMIRKS 逆反応ルール適用（フラグメント正規化・BFS リーク対策含む）
- [x] 2.x A* 探索エンジン（優先度キュー・クローズドリスト・縮退フィルタ）
- [x] 3.x SA Score ヒューリスティック + ビームサーチ
- [x] 4.x rayon 並列化（WASM では逐次フォールバック）
- [x] 5.x Python バインディング（PyO3 + maturin）
- [x] 6.x WASM ビルド（wasm-pack, ~500 KB）
- [x] 7.x ベンチマーク CLI（renkin-bench）
- [x] 8.x ユニットテスト 45件 / ルール 21→31件 / BB 46→160件（WASM）/ 463件（CLI）
- [x] 9.x WASM ブラウザデモ / プリセット 12分子（全解決）
- [x] 10.x グラフベース suzuki_retro/amide_cleavage/boc_cbz_retro / HashMap O(1) BB インデックス
- [x] 11.x crates.io / PyPI / npm 公開 / GitHub Actions CI+Release
- [x] 12.x MkDocs ドキュメントサイト / GitHub Pages WASM プレイグラウンド（i18n EN/JA/ZH）
- [x] README.md / README_ja.md 更新
- [x] tasks/comparison_report.md 競合比較レポート作成

---

## Phase 13: USPTO-50k 正式ベンチマーク 🔴 高優先

### 現状スナップショット（2026-06-22）

| 条件 | BB 数 | ルール数 | 成功率 | avg ms |
|---|---|---|---|---|
| depth=2, beam=20（旧） | 463 | 21 | 5.0%（25/500） | 79.3 ms |
| depth=2, beam=20（新ルール） | 463 | 31 | 5.6%（28/497） | 76.3 ms |
| depth=3, beam=50（新ルール） | 463 | 31 | 10.3%（51/497） | 312 ms |
| depth=3, beam=50 全件 | 463 | 31 | **7.5%（366/4907）** | 305 ms |
| depth=5, beam=50, top-500 | 463 | 314 | **47.2%** | — |
| depth=5, beam=100, Phase A | 463 | 314 | **71.0%（100mol確認）** | — |

AiZynthFinder 参考値: ~45-53%（Genheden 2020 論文値、depth≤5, eMolecules 6M BB, 50k テンプレート）
※ 条件が異なるため直接比較不可。matched-condition 実験は未実施。

### タスク
- [x] **13.0** Phase A 完了: BB 46→160（WASM）、ルール 21→31、depth=3 で 10.3% 達成
- [x] **13.1** USPTO-50k 全件評価（4907件、depth=3, beam=50）
  - 結果: 7.5%（366/4907）、avg 305ms、depth 分布 0:2/1:66/2:133/3:165
  - `tasks/comparison_report.md` 更新済み
- [x] **13.2** chematic issues #13/#14 修正確認
  - #13（BFS leakage）: v0.4.12 で修正済み ✅
  - #14（non-deterministic canonical SMILES）: v0.4.12 で修正済み ✅
- [ ] **13.3** 論文・README にベンチマーク結果を掲載

---

## Phase 14: 自動テンプレート抽出 ✅ 完了

### 目標
USPTO-50k 訓練セット（40,008件）からアトムマッピング済み反応を使って SMIRKS テンプレートを自動抽出 → ルール数 31 → 数百〜数千件

### 結果
- rdchiral で USPTO-50k 40,008 件からテンプレート抽出完了
- top-500: 283 件が chematic 互換（`parse_smarts` 検証済み）→ 314 ルールで統合
- top-5000: `data/templates_extracted_5000.smi` 抽出完了
- ベンチ結果: depth=3 で 38.2%、depth=5 で 47.2%（全件、top-500 ルール）

### タスク
- [x] **14.1** アトムマッピング済み訓練データ取得
  - `bigchem/uspto_reaction_smiles` (HF) 使用
- [x] **14.2** rdchiral でテンプレート抽出スクリプト作成
  - `scripts/extract_templates.py` — 上位 N テンプレート（使用頻度順）を出力
- [x] **14.3** 抽出テンプレートを chem_env.rs に統合（ファイルロード対応）
  - `data/templates.smi` 形式でロード、`default_rules()` を拡張
  - `--templates` フラグを CLI・ベンチ両方に実装済み
- [x] **14.4** 抽出テンプレートで USPTO-50k 再評価
  - depth=5, beam=50, top-500 で 47.2% 達成
- [x] **14.5** top-5000 テンプレート抽出完了
  - `data/templates_extracted_5000.smi` 出力済み（Phase A との組み合わせ検証待ち）

---

## Phase 15: 立体化学対応 🟡 一部残

- [x] **15.0** chematic #20（tetrahedral @/@@）: v0.4.13 で修正済み
- [x] **15.1** RENKIN 側の tetrahedral @/@@ 統合（phase15_stereo テストモジュール、v0.1.3）
- [x] **15.2** `@`/`@@` SMIRKS アトムマップ対応（`parse_smarts_accepts_atom_maps` 拡張）
- [x] **15.3** ステレオ保持テスト追加（`stereo_transferred_to_product` / `both_stereo_templates_are_enantiomer_selective` など）
- [x] **15.4a** E/Z filter（point 1）: chematic 0.4.15 で修正済み → 0.4.16 で RENKIN に採用、テスト 3 本追加（v0.1.4）
- [ ] **15.4b** E/Z transfer（point 2）: chematic 未実装（follow-up）
- [ ] **15.4c** E/Z create（point 3）: chematic 未実装（follow-up）

---

## Phase 16: 大規模 Building Blocks DB ✅ 完了・非優先化

### 結果（2026-06-22）
- eMolecules 4.4M 試験完了
- 463 BB 単独 25% → 463 + eMolecules 26%（**+1pp のみ**）
- **結論**: BB 数より BB のキュレーション品質が重要。eMolecules 単独では基本試薬が不足しており大幅向上は得られない。

### タスク
- [x] **16.1** eMolecules フリーティア（4.4M 分子）のダウンロードと前処理
  - `scripts/prepare_emolecules.py` — SMILES 正規化・重複除去・フィルタリング実施
- [x] **16.2** 大規模 BB DB での USPTO-50k 再評価
  - 結果: +1pp のみ → 非優先化決定
- [ ] **16.3** BB DB サイズ別ベンチマーク比較（参考記録として保留）
- [x] **16.4** WASM 用にキュレーション済み BB セット（160件）を維持しつつ CLI は大規模 DB 使用（対応済み）

---

## Phase 17: chematic Upstream 対応 ✅ 完了

詳細: `tasks/chematic_requests.md` を参照

- [x] **17.1** Issue #13（BFS leakage）: v0.4.12 で修正済み ✅
  - `cargo test issue13_bfs_leakage_check -- --nocapture` でパス確認
- [x] **17.2** Issue #14（non-deterministic canonical SMILES）: v0.4.12 で修正済み ✅
  - eMolecules ブロッカー解消（+1pp のみだったため非優先化）
- [x] **17.3** Issue #18（bracket atom notation）: 修正済み ✅
- [x] **17.4** Issue #19（parse_smarts atom-map）: v0.4.14 で修正済み ✅
- [x] **17.5** Issue #20（tetrahedral @/@@）: v0.4.13 で修正済み ✅ → Phase 15 で RENKIN 側統合予定
- [x] **17.6** Issue #21（E/Z `/\` in SMIRKS）: 0.4.15 で filter（point 1）修正済み ✅ → 0.4.16 で RENKIN に採用
  - transfer（point 2）/ create（point 3）は chematic follow-up → Phase 15.4b/c

---

## インフラ・保守

- [x] **I1** GitHub Pages デプロイ（docs.yml）稼働中
  - URL: https://kent-tokyo.github.io/renkin/playground/
- [x] **I2** CI（fmt/clippy/test 60件）グリーン
- [ ] **I3** PyPI / npm / crates.io のトークンローテーション
- [x] **I4** `src/trace_test.rs` のデバッグテストは `#[ignore]` 相当で分離済み

---

## Phase 18: 精度向上（AiZynthFinder 上限 53% 超えを目指す）

現状: **78.0%（全件確定）**、Phase B（ONNX スコアラー）準備中
目標: **80%+**（Phase B 後）

### 成功率推移（beam=100）
```
7.5% → 27.8% → 38.9% → 47.2% → 54.8% → 71.0%（100mol確認）
 31r    222r    222r    314r    314r    314r
 d=3    d=3     d=5     d=5     d=5     d=5 + beam=100 + Phase A
```

### 戦略

#### 18.1 テンプレート増強 ✅ 完了
- [x] top-500 テンプレート抽出（283 件 chematic 互換）→ 314 ルールで統合済み
- [x] top-5000 テンプレート抽出完了（`data/templates_extracted_5000.smi`）
- [x] chematic #18 修正済み: bracket atom 問題解消 ✅
- [x] chematic #19 修正済み（v0.4.14）: `parse_smarts` atom-map 対応 ✅
- [ ] top-5000 × Phase A 全件ベンチ（さらなる向上確認）

#### Phase A テンプレート頻度重み付け ✅ 完了・効果検証済み
- [x] `RetroRule.weight = ln(count+1)` 実装
- [x] `step_cost -= template_bonus(0〜0.2)` 実装
- [x] 効果確認: 52% → 71%（+19pp、100mol 対照実験）
- [ ] Phase A 全件ベンチマーク確定待ち（目標 65%+）

#### 18.2 SA スコアヒューリスティック改善 — Phase A で代替解決
- Phase A（スコアリング改善）の恩恵で SA スコア改善効果を確認
- 個別調整は引き続きバックログとして保持
  - [ ] 現行の `h = Σ(1 + 0.5·(sa−1)/9)` を実測値でキャリブレーション
  - [ ] depth ペナルティの調整（長経路を過度に嫌わない）

#### 18.3 BB セットのキュレーション強化（+1〜2% 期待）
- [x] USPTO-50k テスト失敗分析: どの BB が不足しているか特定（2026-06-23, `tasks/phase18_bb_analysis.md`）
  - 未解決 1077 件の主要ブロッカー:
    - CF3（10.8%）: Ar-OCF3 系と ArCF3 amine/halide が不足 → **BB +24 件追加済み**（v0.1.4）
    - スルホン/スルホンアミド（10.1%）: retro ルール + BB 追加が必要（未着手）
    - N が多い複素環（+14.6 pp）: ヘテロ環 BB 強化が必要（未着手）
    - N-oxide/ニトロ基（4.5%）: retro-nitro ルール追加が必要（未着手）
  - E/Z 分子（2.9%）の解決率は 65.5%（非 E/Z は 78.5%）— chematic 0.4.16 の filter が影響
  - Br は解けやすいマーカー（−5.4 pp）— Suzuki/Heck の離脱基として機能
- [x] CF3 系 BB 追加: 538 → 562 件（+24 件、OCF3 系 10 + ArCF3 amine/halide 8 + CH2CF3 3 + 他）
  - ベンチ計測中（`data/bench_chunks_cf3`）
- [ ] スルホンアミド BB / retro ルール追加（未着手）
- [ ] ヘテロ環 BB 強化（未着手）
- [ ] eMolecules から基本試薬のみ手動抽出（N、O、Cl2 など）して BB セットに追加

#### Phase B ONNX テンプレート関連性モデル（完了・リバート済み）
- [x] `scripts/train_template_scorer.py` で MLP 訓練（Morgan FP → template prob）
- [x] `tract` クレートで Rust ONNX 推論（feature = `nn-scoring`）（※ `ort` ではなく `tract` 採用）
- [x] A* g 値に NN ボーナスを統合（2026-06-28）
  - `scorer.rs`: `rule_bonuses()` — ONNX logit を min-max 正規化 → `[0, NN_BONUS_SCALE=0.15]` に変換
  - `search.rs`: `nn_bonus_map` を事前計算、`g: node.g + step_c - nn_b` でテンプレート選択をバイアス
  - freq bonus（≤0.2）と合算で最大 0.35/ステップ < min step_cost 1.0（スケールは安全範囲内）
  - バグ修正: `from_path` の `with_input_fact` を削除（dynamic batch と競合して `into_optimized` が失敗していた）
- [x] 効果確認（2026-06-28）: **−8.4 pp（78.0% → 69.6%、全件 4907 分子）、逆効果と判定・リバート済み**
  - `data/bench_chunks_phaseB2_b100`（depth=5, beam=100, scorer=template_scorer.onnx）
  - 原因仮説: min-max 正規化がロジット差の小さい場合にノイズを増幅 / `top_k_indices` リランク + g 値割引の二重影響
  - 対処: `rule_bonuses()`/`raw_logits()` 削除・`nn_bonus_map` 削除・`g: node.g + step_c` に戻した（`top_k_indices` リランクのみ維持）
- ※ 旧 18.4（GNN スコアリング）の後継として位置付け

### ポジション目標
```
現在: 78.0%（Phase A baseline、リバート後の現状）
次手: Phase B 別アプローチ検討（softmax 温度付き重み付けサンプリング、beam 内での動的テンプレート再選択など）
目標: 80%+
```

### ⚠️ 評価の限界（Phase 20 で検証予定）
- 現在の比較はすべて「自社計測 vs 競合論文値（2019-2022）」であり matched-condition 実験なし
- LocalRetro/GLG の数値は単ステップ top-1 精度（≠ 多段階経路探索成功率）— 直接比較不可
- Phase A は強い in-domain バイアス（訓練分布 = テスト分布）— OOD 性能は未検証

---

## Phase 20: 評価の妥当性検証 🔴 高優先（2026-06-22 追加）

現在の 78.0% という数値の信頼性を高めるための検証タスク。

- [x] **20.1** LocalRetro/GLG の指標確認: 原論文を再読し「単ステップ精度」か「多段階経路探索」かを明記。comparison_report.md の比較表から誤った比較を除去済み ✅
- [ ] **20.2** matched-condition 実験: RENKIN の 537 BB セットで AiZynthFinder を走らせ、アルゴリズム差を切り離す
- [x] **20.3** OOD 評価（2026-06-25 実施）:
  - データ: ChEMBL Phase 4 承認薬 500 件（3,475 件から MW 150-700/HAC 10-60 でフィルタ後 1,915 件→500 件サンプリング）
  - 結果: **81.8%（409/500）** — USPTO-50k の 78.1% を **+3.7 pp 上回る**
  - 解釈: in-domain bias はなく、承認薬でも同等以上に機能する
  - 未解決 91 件の特徴: N が多い（+17.5 pp）、F が多い（+11.5 pp）— USPTO と同じパターン
  - スクリプト: `scripts/fetch_chembl_approved.py`、データ: `data/chembl_approved_ood.smi`
- [ ] **20.4** テンプレート制約厳格化実験: `simplify_smirks()` の D/H0/+0 除去が成功率に与える影響を定量化
- [x] **20.5** 立体化学影響分析（2026-06-23 実施）:
  - テスト分子 4907 件中 E/Z マーカーあり: **144 件（2.9%）**（※旧メモの ~21% は誤り）
  - top-5000 テンプレート中 E/Z あり: 209 件（4.2%）
  - E/Z 分子の解決率: **65.5%**（19/29、v0.1.4 速報）vs 非 E/Z: **78.5%**（762/971）
  - E/Z フィルタが有効になったことで E/Z 分子は若干解きにくくなった可能性あり（全件結果待ち）

---

## Phase 19: Rust 内部最適化 ✅ 完了（2026-06-22 追加）

コアエンジンのホットパスを最適化し、スループット向上・メモリアクセス削減を実現。

- [x] **Opt-1** `split_fragments` の冗長呼び出し削減（`chem_env.rs:444`）
  - 冗長な `canonical_smiles` × 2 + `parse` を 1 回に削減
- [x] **Opt-2** `is_bb` に HashSet 直接ルックアップのファストパス追加
  - VF2 グラフ同型フォールバックは維持（正確性は損なわない）
- [x] **Opt-3** `RetroRule.required_elements: u64` bitset によるプリスクリーニング
  - `required_elements_from_smirks()` でロード時に計算
  - `elem_mask_from_smiles()` で照合 → `apply_retro` 前に非候補テンプレートを即除外
