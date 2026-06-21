# RENKIN - Todo

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

### 現状スナップショット（2026-06-20）

| 条件 | BB 数 | ルール数 | 成功率 | avg ms |
|---|---|---|---|---|
| depth=2, beam=20（旧） | 463 | 21 | 5.0%（25/500） | 79.3 ms |
| depth=2, beam=20（新ルール） | 463 | 31 | 5.6%（28/497） | 76.3 ms |
| depth=3, beam=50（新ルール） | 463 | 31 | 10.3%（51/497） | 312 ms |
| depth=3, beam=50 全件 | 463 | 31 | **7.5%（366/4907）** | 305 ms |

AiZynthFinder 参考値: ~45-53%（depth≤5, eMolecules 6M BB, 50k テンプレート）

### タスク
- [x] **13.0** Phase A 完了: BB 46→160（WASM）、ルール 21→31、depth=3 で 10.3% 達成
- [x] **13.1** USPTO-50k 全件評価（4907件、depth=3, beam=50）
  - 結果: 7.5%（366/4907）、avg 305ms、depth 分布 0:2/1:66/2:133/3:165
  - `tasks/comparison_report.md` 更新済み
- [ ] **13.2** chematic issues #13/#14 修正待ちモニタリング
  - 毎回 `cargo update chematic` で確認 → 修正確認テストは `trace_test.rs` に実装済み
- [ ] **13.3** 論文・README にベンチマーク結果を掲載

---

## Phase 14: 自動テンプレート抽出 🟡 中優先

### 目標
USPTO-50k 訓練セット（40,008件）からアトムマッピング済み反応を使って SMIRKS テンプレートを自動抽出 → ルール数 31 → 数百件

### 前提条件
- アトムマップ済み USPTO-50k 訓練データ取得（Hugging Face の別データセット or rxnmapper 実行）
- rdchiral（Python）でテンプレート抽出
- 抽出したテンプレートを chematic の `run_reactants` に食わせてフィルタリング

### タスク
- [ ] **14.1** アトムマッピング済み訓練データ取得
  - 候補: `bigchem/uspto_reaction_smiles` (HF), `rxnmapper` で自動マッピング
- [ ] **14.2** rdchiral でテンプレート抽出スクリプト作成
  - `scripts/extract_templates.py` — 上位 N テンプレート（使用頻度順）を出力
- [ ] **14.3** 抽出テンプレートを chem_env.rs に統合（またはファイルロード対応）
  - `data/templates.smi` 形式でロード、`default_rules()` を拡張
- [ ] **14.4** 抽出テンプレートで USPTO-50k 再評価
  - 目標: 20%+ 解決率

---

## Phase 15: 立体化学対応 🟢 低優先

- [ ] **15.1** chematic の CIP ステレオ対応状況確認（issues #13/14 との依存関係）
- [ ] **15.2** `@`/`@@` SMIRKS アトムマップ対応
- [ ] **15.3** ステレオ保持テスト追加

---

## Phase 16: 大規模 Building Blocks DB 🟡 中優先

- [ ] **16.1** eMolecules フリーティア（~3M 分子）のダウンロードと前処理
  - `scripts/prepare_emolecules.py` — SMILES 正規化・重複除去・フィルタリング（MW<500）
  - DL先: https://downloads.emolecules.com/free/
- [ ] **16.2** 大規模 BB DB での USPTO-50k 再評価
  - 想定: 463件→3M で 10%→25-35% 解決率向上
- [ ] **16.3** BB DB サイズ別ベンチマーク比較（463 / 10k / 100k / 3M）
- [ ] **16.4** WASM 用にキュレーション済み BB セット（160件）を維持しつつ CLI は大規模 DB 使用

---

## Phase 17: chematic Upstream 対応 🔴 最優先ブロッカー（eMolecules BB 拡充の前提）

詳細: `tasks/chematic_requests.md` を参照

- [ ] **17.1** Issue #13（BFS leakage）修正確認・アップストリーム投稿
  - `cargo update chematic` → `cargo test issue13_bfs_leakage_check -- --nocapture`
  - 修正後: SMIRKS ルールを増やしやすくなる（現状はグラフ実装で回避中）
- [ ] **17.2** Issue #14（non-deterministic canonical SMILES）修正確認 ← **eMolecules 拡充のブロッカー**
  - 実験: eMolecules 4.4M BB で USPTO-50k → ≈0%（同一分子が複数 canonical 形を持つため照合失敗）
  - 例: `CC(=O)O` → `O=C(C)O`, `OC(C)=O` → `OC(C)=O`（どちらも酢酸だが一致しない）
  - `cargo test issue14_canonical_smiles_truly_canonical -- --nocapture`
- [ ] **17.3** Feature request: 立体化学 SMIRKS 対応（`@`, `@@`）投稿
- [ ] **17.4** Feature request: `run_reactants` の atom-correct モード投稿

---

## インフラ・保守

- [x] **I1** GitHub Pages デプロイ（docs.yml）稼働中
  - URL: https://kent-tokyo.github.io/renkin/playground/
- [x] **I2** CI（fmt/clippy/test 45件）グリーン
- [ ] **I3** PyPI / npm / crates.io のトークンローテーション
- [x] **I4** `src/trace_test.rs` のデバッグテストは `#[ignore]` 相当で分離済み

---

## Phase 18: 精度向上（AiZynthFinder 上限 53% 超えを目指す）

現状: **47.2%**（depth=5、314 ルール）
目標: **53%+**（AiZynthFinder 最大値を超え、LocalRetro 水準へ）

### 戦略

#### 18.1 テンプレート増強（+2〜5% 期待）
- [ ] top-1000 テンプレート抽出・ベンチマーク
  - `python3 scripts/extract_templates.py --top 1000 --output data/templates_extracted_1000.smi`
  - chematic 互換テンプレート数の上限を確認
- [ ] chematic #18 修正後: bracket atom 問題が解消 → より多くのテンプレートが正確に動作
- [ ] chematic #19 修正後: `parse_smarts` で atom-map 対応 → 検証の高速化

#### 18.2 SA スコアヒューリスティック改善（+1〜3% 期待）
- [ ] 現行の `h = Σ(1 + 0.5·(sa−1)/9)` を実測値でキャリブレーション
- [ ] depth ペナルティの調整（長経路を過度に嫌わない）
- [ ] 合成可能性以外の因子（MW、ring count）の追加

#### 18.3 BB セットのキュレーション強化（+1〜2% 期待）
- [ ] USPTO-50k テスト失敗分析: どの BB が不足しているか特定
  - `data/bench_chunks_d5_t500/` の未解決分子を集計
  - 頻出するフラグメントを新 BB として追加
- [ ] eMolecules から基本試薬のみ手動抽出（N、O、Cl2 など）して 463 BB セットに追加

#### 18.4 GNN スコアリング（長期、大幅向上の可能性）
- [ ] AiZynthFinder / Retro* の NN スコアリング方式を調査
- [ ] RENKIN の A* ヒューリスティックを NN スコアで補強する設計を検討
- [ ] PyO3 経由で Python ML モデルを呼び出す統合プロトタイプ

### ポジション目標
```
現在: 47.2%（AiZynthFinder 下限 45% を超過）
目標: 53%+（AiZynthFinder 上限を超え LocalRetro 水準へ）
長期: 58%+（GLG 水準、GNN 統合後）
```
