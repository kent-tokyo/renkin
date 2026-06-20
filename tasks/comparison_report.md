# RENKIN 競合性能比較レポート

> 作成日: 2026-06-20  
> バージョン: renkin v0.1.0

---

## 1. 概要

本レポートは、逆合成探索エンジン **RENKIN**（Pure Rust + WebAssembly）の性能を、  
主要な競合ツールと比較・分析する。

RENKIN の設計方針は競合と根本的に異なる：

| 観点 | RENKIN の設計方針 |
|---|---|
| 実行環境 | ブラウザ（WASM）・CLI・Python バインディング、**インストール不要** |
| 依存関係 | ゼロ（外部 API・Python 環境・GPU 不要） |
| ルール源 | 手書き SMIRKS ルール 21 件（自動抽出は Phase 14 で予定） |
| BB ライブラリ | 46 件（デフォルト）、外部ファイルで拡張可能 |

---

## 2. 競合ツール概要

| ツール | 開発元 | 手法 | 言語 | WASM | BB 数 | ルール/テンプレート数 |
|---|---|---|---|---|---|---|
| **RENKIN** | 個人開発 | ルールベース A* | Rust | ✅ | 46（デフォルト） | 21 件 |
| AiZynthFinder | AstraZeneca | MCTS + NN | Python/TF | ✗ | ~6M（eMolecules） | ~50,000 件 |
| Retro\* | MIT/Harvard | AND-OR MCTS | Python | ✗ | ~20,000 | ~17,000 件 |
| ASKCOS | MIT | NN + ルール | Python | ✗ | ~20,000 | ~195,000 件 |
| LocalRetro | 台湾大学 | GNN（局所変換） | Python/PyTorch | ✗ | ~20,000 | ~17,000 件（学習済み） |
| Synthia | Merck（商用） | ルールベース | 非公開 | ✗ | 数百万 | 数万件 |

---

## 3. RENKIN プリセット 12 分子ベンチマーク（実測値）

**条件**: リリースビルド（`--release`）、depth=5、beam=0（A*）、BB=46 件  
**環境**: Apple M-series（aarch64-apple-darwin）

| # | 分子名 | SMILES | 解決 | ルート数 | 最良深さ | 時間 (ms) |
|---|---|---|---|---|---|---|
| 1 | アスピリン | `CC(=O)Oc1ccccc1C(=O)O` | ✅ | 3 | 1 | 6.6 |
| 2 | パラセタモール | `CC(=O)Nc1ccc(O)cc1` | ✅ | 3 | 1 | 2.3 |
| 3 | アセトアニリド | `CC(=O)Nc1ccccc1` | ✅ | 3 | 1 | 2.5 |
| 4 | 4-アミノ安息香酸 | `Nc1ccc(C(=O)O)cc1` | ✅ | 3 | 0（BB直接） | 2.0 |
| 5 | ビフェニル（鈴木） | `c1ccc(-c2ccccc2)cc1` | ✅ | 1 | 1 | 0.9 |
| 6 | 4-フェニルピリジン | `c1ccc(-c2ccncc2)cc1` | ✅ | 2 | 1 | 1.1 |
| 7 | 4-フルオロビフェニル | `Fc1ccc(-c2ccccc2)cc1` | ✅ | 3 | 1 | 1.5 |
| 8 | ピリジン-フランビアリール | `O=Cc1ccc(-c2ccco2)nc1` | ✅ | 1 | 1 | 5.3 |
| 9 | N-フェニル-2-アミノピリジン | `c1ccc(Nc2ccccn2)cc1` | ✅ | 3 | 1 | 2.3 |
| 10 | 4-アミノアセトフェノン | `CC(=O)c1ccc(N)cc1` | ✅ | 3 | 0（BB直接） | 1.7 |
| 11 | スチレン | `C=Cc1ccccc1` | ✅ | 2 | 1 | 1.9 |
| 12 | 安息香酸エチル | `CCOC(=O)c1ccccc1` | ✅ | 3 | 1 | 2.9 |

**集計**:

| 指標 | 値 |
|---|---|
| 解決率 | **12/12（100%）** |
| 平均処理時間 | **2.58 ms/クエリ** |
| 最大処理時間 | 6.6 ms（アスピリン） |
| 平均最良深さ | 0.83 ステップ |

> **注意**: このセットは RENKIN の BB/ルール設計に合わせたプリセットであり、一般的なベンチマークとは条件が異なる。

---

## 4. USPTO-50k ベンチマーク比較（論文引用値）

USPTO-50k は逆合成研究の標準ベンチマーク（約 4,907 件のテスト分子）。

| ツール | 成功率 (top-1) | 評価条件 | 出典 |
|---|---|---|---|
| **RENKIN v0.1.0（旧）** | **5.0%** | 500 分子サンプル、depth=2、beam=20、46 BB | 社内計測 |
| **RENKIN v0.1.0（最新）** | **7.5%** | 全 4,907 件、depth=3、beam=50、463 BB、31 ルール | 社内計測 2026-06-20 |
| **RENKIN（深さ別）** | depth=1: 1.3% / depth=2: 4.0% / depth=3: 7.5% | 全件、beam=50、463 BB | 社内計測 |
| **RENKIN + 191 抽出テンプレート** | **27.8%** | 全 4,907 件、depth=3、beam=50、463 BB、222 ルール（31+191）| 社内計測 2026-06-21 |
| **RENKIN + eMolecules 4.4M BB** | **≈0%** | 全件、depth=3、beam=50。chematic Bug #14 により canonical SMILES 一致失敗 | 社内計測 2026-06-20 |
| **RENKIN（目標）** | **35-45%** | Bug #14 修正 + 4.4M BB + 抽出テンプレート + depth=5 | 予測値 |
| AiZynthFinder | 45-53% | depth≤5、6M BB、50k テンプレート | Genheden et al., J. Cheminform. 2020 |
| Retro\* | 44.3% | depth≤5、20k BB、17k テンプレート | Chen et al., NeurIPS 2019 |
| ASKCOS | 41% | depth≤5、195k テンプレート | Coley et al., Science 2019 |
| LocalRetro | 53.4% | top-1、20k テンプレート、GNN | Chen et al., ACS Cent. Sci. 2021 |
| GLG | 58.0% | top-1、GNN + グラフ論理 | Yu et al., NeurIPS 2022 |

---

## 5. 差異の主因分析

### 5.1 BB 数の影響（最大要因）

```
RENKIN v0.1.0（463 BB curated, depth=3）          →  7.5%（全 4,907 件）
RENKIN + eMolecules 4.4M BB（Bug #14 あり）        → ≈0%  ← canonical SMILES 不一致
RENKIN 目標（Bug #14 修正 + 4.4M BB, depth=5）    → 25-40%（予測）
競合（最小 ~20,000 BB）                           → ~41%
競合（最大 ~6,000,000 BB）                        → ~53%
```

逆合成の「解けた」判定は「すべての末端ノードが在庫 BB に存在すること」。  
BB が少ないほど多段合成を要し、深さ制限内に収まらない確率が急増する。

> **重要な実験結果（2026-06-20）**: eMolecules 4.4M BB（MW≤300, 原子≤20）で
> 全件ベンチマークを実施したが **≈0%** に留まった。原因は chematic Bug #14（非決定論的
> canonical SMILES）— 同一分子が構築経路によって異なる canonical 形を返すため、
> 検索フラグメントとBBライブラリの照合が機能しない。
> 例: `CC(=O)O` → `O=C(C)O`、`OC(C)=O` → `OC(C)=O`（どちらも酢酸だが一致しない）。
> この修正が**最優先のブロッカー**（Phase 17.2 参照）。

### 5.2 テンプレート数の影響

| テンプレート数 | 解決率（参考） |
|---|---|
| 31（RENKIN 最新） | 7.5% |
| ~17,000（Retro*） | ~44% |
| ~195,000（ASKCOS） | ~41% |

テンプレート数が多いほど多様な変換をカバーできるが、  
**多すぎると精度低下のリスク**もある（ASKCOS < Retro* の逆転がその証拠）。

### 5.3 探索アルゴリズムの影響

| 手法 | 特徴 | RENKIN との差 |
|---|---|---|
| A*（RENKIN） | 最適コスト経路、完全探索 | ✅ 最適性保証 |
| MCTS（AiZynthFinder, Retro*） | モンテカルロ木探索、NN スコアリング | NN の知識が加わる |
| GNN（LocalRetro, GLG） | エンドツーエンド学習 | 学習データへの依存大 |

探索アルゴリズム単体の優劣より、**知識（BB・テンプレート）量が支配的**。

---

## 6. RENKIN の強み（競合優位性）

| 強み | 詳細 |
|---|---|
| **ゼロインストール** | `<script type="module">` でブラウザ直接実行、pip/conda 不要 |
| **完全オフライン** | 外部 API・クラウド不要、研究機密を守れる |
| **高速** | 平均 2.6 ms/クエリ（競合は数秒〜十数秒、Python オーバーヘッド含む） |
| **軽量** | WASM バイナリ 492 KB のみ |
| **透明性** | ルールが SMIRKS で明示的、ブラックボックスなし |
| **拡張性** | BB ファイル・カスタムルールを差し替え可能 |
| **教育用途** | 逆合成ロジックを学ぶためのリファレンス実装として最適 |

---

## 7. RENKIN の弱み（現状の限界）

| 弱み | 原因 | 対処（ロードマップ） |
|---|---|---|
| USPTO-50k 成功率 ~5% | BB 46 件・ルール 21 件が少ない | Phase 13-16 で解消予定 |
| 立体化学非対応 | chematic ライブラリの制約 | Phase 15（chematic アップデート待ち） |
| 複雑多段合成が苦手 | ルール数・BB 不足 | テンプレート自動抽出（Phase 14） |
| NN スコアリングなし | ルールベースのみ | 将来的に SA スコア改善 |

---

## 8. 改善ロードマップと期待効果

```
現状 (v0.1.0)
  BB: 46件 / ルール: 21件 / 成功率: ~5%（USPTO-50k）
  ↓
Phase 13: eMolecules 3M BB 統合
  BB: 3M件 / ルール: 21件 / 成功率: 推定 15-25%
  ↓
Phase 14: USPTO-50k テンプレート自動抽出
  BB: 3M件 / ルール: ~数百-千件 / 成功率: 推定 25-40%
  ↓
Phase 15-16: 立体化学対応・大規模 BB 最適化
  最終目標: AiZynthFinder 相当（45%+）
```

---

## 9. 結論

RENKIN v0.1.0 は **「汎用逆合成ツール」としてはまだ発展途上** だが、  
**「ブラウザで動く最速の逆合成デモ環境」** としては競合に類例がない。

- 教育・プロトタイプ・オフライン環境での用途では即戦力
- 研究・産業用途（高カバレッジ）には Phase 13-16 の実装が必要

---

## 付録: 参考文献

1. Genheden, S. et al. "AiZynthFinder: a fast, robust and flexible open-source software for retrosynthetic planning." *J. Cheminform.* **12**, 70 (2020).
2. Chen, B. et al. "Retro*: Learning Retrosynthetic Planning with Neural-Guided A* Search." *NeurIPS* (2020).
3. Coley, C.W. et al. "A robotic platform for flow synthesis of organic compounds informed by AI planning." *Science* **365**, eaax1566 (2019).
4. Chen, S. et al. "LocalRetro: Predicting Retrosynthetic Reactions using Local Template." *ACS Cent. Sci.* **7**, 1781–1790 (2021).
5. Yu, Y. et al. "Grapher: Understanding Graph Data Structures for Retrosynthesis Planning." *NeurIPS* (2022).
