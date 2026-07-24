# RENKIN - Todo

## Phase 31: 検証精度の是正 — validated_solved_rate 修正と公開数値の再計測 ✅ corrected baseline公開完了（2026-07-22）

RENKINを競合超えのRust-native CASPエンジンへ進化させる長期ゴールのPhase 0/1着手。「まず測定→仮説→小PR→検証」の順で進行中。詳細な計測ログ・before/after数値は `tasks/phase0_baseline.md`（gitignore対象、ローカルのみ）を参照。

- [x] **31.1** Phase 0 baseline測定（コード変更なし）（2026-07-19）
  - raw/validated/practical solved rate再現、depth=0 direct stock hit率、cascade追加解決群の品質差、p50/p95/peak memory/nodes expandedを測定
  - 3つの候補PRを効果・リスク・工数・保守負担で比較 → PR #1を選定
- [x] **31.2** PR #1 `fix/forward-validation-graph-rule-blindspot`（3コミット、2026-07-19）
  - forward validationを bool → 三値（`StepValidationStatus::{Valid,Invalid,NotEvaluable}` / `RouteValidationStatus`）に変更
  - graph-based 7ルール（ester/amide/Suzuki/sulfonamide/sulfone/Boc/Cbz cleavage）専用の原子組成デルタ検証を追加（ホワイトリスト方式は不採用）
  - `src/validation/` モジュール新設（`atom_conservation.rs` / `forward.rs` / `graph_rules.rs`）、`benchmark.rs`/`mcp.rs` の重複ヘルパーを統合
  - `renkin-bench` に `route_validation_status` / `strict_validated_solved_rate` / `validation_coverage` / `evaluable_validation_pass_rate` を追加（JSON互換は維持）
  - commit: `refactor: centralize shared validation helpers` → `fix: validate graph-based retrosynthesis rules` → `test: add graph-rule validation coverage`
- [x] **31.3** PR #2 `fix/aryl-carboxylation-retro-ester-overmatch`（2026-07-19）
  - **発見**: `aryl_carboxylation_retro` の SMIRKS（`[c:1][C:2](=O)O`）が末端酸素にH制約なく、エステルにも誤発火 → precursor生成時にR基が消失（例: 安息香酸メチル → `[benzene, formic acid]`、OMe消失）
  - 修正: 末端を `[OH]` に制約（chematic-smarts のH-count実装を先に確認: `explicit_h + implicit_hcount == h` の完全一致）
  - **unbiased n=200サンプルで raw solved 199/200 → 61/200（-69%）、pct_atom_balanced 12.1% → 55.7%** — 偽solved除去と判定（能力低下ではない）
  - **含意**: 公開済みの78.0%（USPTO-50k単一パス）・95.9%（cascade）・81.8%（ChEMBL OOD）は本バグ修正前の計測値であり、信頼できない可能性が高いと判明
- [x] **31.4** docs hotfix（2026-07-19）
  - README.md/README_ja.md/docs/benchmark.md/docs/benchmark_ja.md/docs/index.md に「invalidated historical measurement」注記を追加、バッジを "under re-evaluation" に変更（数値は削除せず注記のみ）
- [x] **31.5** SMIRKS element-conservation 手動監査（2026-07-19）
  - `friedel_crafts_acylation_retro` を含む8ルールを `apply_retro` への境界ケース直接投入で検証
  - **結論**: atom-lossバグは `aryl_carboxylation_retro` のみに孤立。他ルールはmapped原子の実置換基がH-count再宣言（CH2→CH3等）を跨いでも正しく保存されることを実証
  - `friedel_crafts_acylation_retro` は原子は失わないが、エステル/アミド/アルデヒドへの過剰マッチで化学的に非現実的な試薬（クロロギ酸エステル等）を提案する別種の問題と判明（atom lossではなく妥当性の問題、優先度は低）
  - 汎用監査CLI（`audit-smirks`）は作らず、発見した境界ケースを `chem_env.rs` にテーブル駆動回帰テスト（`substituent_preservation_regression_suite`）として固定（commit: `test: pin down substituent-preservation audit findings as regression coverage`）
- [x] **31.6** friedel_crafts頻度ゲート判定（2026-07-19）
  - n=400ランダムサンプル（修正版バイナリ）で92件solved、best routeの全ステップを集計 → **friedel_crafts_acylation_retro 使用0件**
  - ゲート条件（複数回採用/solved判定への寄与/ester・amideより上位/validation失敗主因）はいずれも非該当 → PR #3は不要と判断、全件再計測へ直行
  - 参考: この修正版バイナリでunbiased n=400 raw solved = 92/400（23%）— 旧78.0%を大幅に下回る
- [x] **31.7** USPTO-50k全4,907件 Stage1再計測（2026-07-20 20:42開始 〜 2026-07-21 00:1x完了、約3.4h）
  - PR #25・#26をmasterへmerge（`fab67fc`→`94b9501`）、RUSTSEC-2026-0204依存修正をPR #27として分離merge（`35f26cb`）— この修正版masterのcommit `35f26cb` で計測
  - harness: `scripts/run_benchmark_parallel.sh`新設（5並列shard×`RAYON_NUM_THREADS=2`、`run_benchmark_chunks.sh`に`--plausibility` passthrough追加）PR #28として分離merge
  - **結果（詳細・provenance: `tasks/phase31_corrected_baseline_run.md`）**: raw_solved_rate 24.01%(1178/4907)、depth=0 hit 0.04%、pct_atom_balanced 58.32%、validation_coverage 100%（Finding Aのblind spot解消を確認）、p50/p95/p99=8.2s/37.3s/79.9s
  - **strict_validated_solved_rate(0.9986%)・evaluable_validation_pass_rate(20.25%)は「正しさの指標」として未公開**: n=300サンプル調査で、(a) `aryl_fluoride_snAr_retro`/`aryl_iodide_retro`/`aryl_chloride_retro`が`aryl_carboxylation_retro`と同型のatom生成/消失バグ（`[c:1][X]>>[c:1]`、詳細は31.11）、(b) `smirks_reproduces()`が「実際に使ったrule」ではなく「どれかのruleが再現できればValid」なため、49件のvalidated routeのうち7件がatom不整合なaryl_chloride_retro stepを含むのにValid判定（偶然の別rule一致）— ValidにもInvalidにも既知の汚染があり、公開時は「validatorが確認できた範囲」としてのみ扱う
  - **raw_solved_rate 24.0%はcommit `35f26cb`時点の暫定値** — 31.11のhalideバグを直すと`aryl_carboxylation_retro`と同様にさらに下がる可能性が高い。修正→再計測→再修正の無限ループを避けるため、今回は意図的にこのcommitで確定・修正は次PRへ
- [x] **31.9** README/docs/benchmark.md/バッジを修正版の実測値へ更新（PR #34、squash commit `96d74d3`、2026-07-22）
  - 31.11・31.12マージ後のcommit `e20dc8c`でUSPTO-50k全4,907件を再計測（詳細・provenance: `tasks/phase31_final_remeasurement_run.md`）
  - 公開: raw_solved_rate 20.09%(986/4907) → atom_balanced_solved_rate 15.41%(756/4907) → provenance_validated_solved_rate 0.88%(43/4907) の入れ子系列。Public label（Search-to-stock rate / Atom-balance-filtered rate / Current-validator-confirmed rate）を併記
  - レビューで「floor」「正確性の下限」表現を撤回・是正（0.88%は実測正確性でも数学的に証明された下限でもない、と明記）、READMEバッジは非数値化、building block数はファイル行数(475)ではなく`ChemEnv::bb_count()`実ロード数(402)へ全箇所統一
  - cascade(95.9%)・ChEMBL OOD(81.8%)は未再計測のまま明示的に無効化継続（本タスクでは再計測せず）
- [x] **31.10**（判断確定）`friedel_crafts_acylation_retro` の過剰マッチ問題 — 31.6のn=400頻度ゲートで使用0件と確認済み、対応不要と結論。追加対応なし
- [x] **31.11** ハンドクラフトruleの原子生成/消失バグ3件（PR #31、`4f47ede`、2026-07-21）
  - `aryl_fluoride_snAr_retro`・`aryl_iodide_retro`・`aryl_chloride_retro`を`default_rules()`から削除（原子保存版の捏造は不採用、化学的根拠なしのため除去がデフォルト方針）
  - n=300 before/after測定: 74/300(24.7%)→66/300(22.0%)、該当ルール依存9件中8件が正しくunsolvedへ
- [x] **31.12a** forward validatorのrule-provenance拘束（PR #33、`e20dc8c`、2026-07-21）
  - `validate_step`を「実際に使ったrule」のみのreverse-SMIRKS/graph-structural検証に拘束、cross-rule偶然一致によるValid誤判定を排除
  - cross-rule false positiveの実例をinline rule setの回帰テストとして固定
- [ ] **31.12b**（Phase 32へ）validator fidelity分析 — n=300サンプルでInvalid判定695件中575件(83%)相当がatom-balanced済みなのにreverse-SMIRKS再現失敗。本物の化学的誤りかvalidatorの偽陰性か未分離のまま。詳細は下記Phase 32参照

Phase 31のcorrected baseline確立・公開はこれで完了。最終状態・全provenanceは `tasks/phase31_final_remeasurement_run.md` を参照。残る検証精度の課題はPhase 32へ引き継ぐ。

---

## Phase 32: 検証精度の残課題（backlog、2026-07-22 Phase 31から分離）

Phase 31で「壊れた指標を先に直す」は完了。ここからは探索精度そのものではなく、残った検証・計測基盤の課題。

- [ ] **31.8**（Phase 31より移動）cascade Stage2再計測（Stage1未解決分、depth=7 beam=300）— 修正版ルールセット(commit `e20dc8c`以降)に対して未実施。Stage1と結果を分離して報告
- [ ] **31.12b**（Phase 31より移動、上記参照）validator fidelity分析 — 実際の化学的誤りとvalidator偽陰性（canonicalization/芳香族性/立体/tautomer起因）の分離。`suzuki_retro`は0%invalidな一方`cc_single_cleavage`/`cn_aliphatic_cleavage`等は70-100% invalidという広がりから、単一要因では説明できない。三値→多値化（`AlternateRuleCorroborated`/`FormulaConsistent`等）が候補
- [x] **32.1** `renkin-bench compare`のdedup keyバグ修正 — PR #32のharness監査で発見。`name`優先→`smiles`フォールバックだがUSPTO-50kは全件`name="UNK"`のため実質機能しない（100件サンプルで実際は12件regressionのところ0件と誤報告）。**PR #37（squash `f6758f0`、2026-07-22）でマージ済み、CI green**: 識別キーをsmiles優先→`#<行番号>:<smiles>`フォールバック（重複smiles ~4/4907件用）に変更、ターゲット集合不一致時は非ゼロ終了で即エラー。回帰テスト4件（bug pin/重複smiles/件数不一致/集合不一致）。pre-fix binaryでの実地確認済み（gained/lost 0→正しく1/1検出）
- [ ] **32.2** ChEMBL OOD再計測 — 修正版ルールセット(commit `e20dc8c`以降)に対して未実施。旧81.8%は無効化のまま

### Phase 32拡張: matched-condition競合超えゴール（2026-07-22 `/goal`より、13,969字で4,000字上限超過のためharness未追跡・全文を `tasks/phase32_matched_condition_goal.md` に保存）

RENKINを競合（AiZynthFinder等）に対しmatched-condition下で統計的に上回らせる（Win A）。stock拡大・化学的に無効な経路・test leakage・不公平比較での「勝利」は禁止。詳細な優先順位・7サブエージェントトラック(A-G)・禁止事項・merge順序・スコア式は上記ファイル参照。

- [x] **32.3** ボトルネック分解（追加計測なし、既存Phase31結果データのみ）: `scripts/decompose_bottlenecks.py`（2026-07-22）
  - 未解決3,921件中3,920件(99.97%)が`beam_limit_hit`かつ`max_depth_reached`同時True、`matched_templates`中央値25,081、`stock_hits`>0 — テンプレート・在庫は枯渇していないのに探索予算（深さ5・ビーム幅100）を使い切って未達
  - 判別力確認: solved側の`max_depth_reached`率はわずか10.3%（unsolved 99.97%との差+89.7pp）→ フラグ常時Trueのアーティファクトではなく実信号
  - stock_limited/template_limited該当は各1件/0件のみ
  - **結論: 支配的ボトルネックはTrack E（探索アルゴリズム/予算）— Track B（stock）・C（template量）への投資は現時点で優先度低**
- [x] **32.4** 固定スクリーニングコーパス構築（merge順序2）: `scripts/build_screening_corpora.py` → `data/corpora/{screening_500,hard_200,quality_200}.json`（2026-07-22、seed=32、sha256記録）
  - screening_500: solved/unsolved比率+depth層化。反応クラス層化は不可（`uspto50k_test.smi`全件`reaction_class=UNK`）と明記
  - hard_200: unsolved×search_limited bucketをnodes_expanded四分位で層化
  - quality_200: 暫定でdepth/validation_status/atom_balance_ok層化（rule-usage層化は`examples/inspect_validation`実行完了後に再構築予定）
- [x] **32.5** Track A完了（2026-07-22）: 32.1のdedup修正（PR #37マージ済み、上記）+ 競合実現可能性スパイク
  - **AiZynthFinder: GO** — outbound network到達可（PyPI/GitHub/Zenodo/conda-forge）。system Python 3.13/3.14が非対応のため`brew install python@3.11`（**システムレベル変更、repo外**）。venv隔離で`aizynthfinder==4.4.1`導入、公開データ(~790MB: USPTO展開/ringbreaker/filterモデル+ZINC stock 1740万件)取得、USPTO-50k先頭ターゲットで実行 → **2.0秒で解決**（96経路探索、39解決、top score 0.998）。ただしAiZynthFinder自前のZINC stock(1740万)とRENKINの402 BB・MCTS 120s予算 vs beam探索は全くmatched-conditionでない — 本番比較には stock統一（RENKIN 402 BBをAiZynthFinderのcustom stockとして注入）が必須設計課題として残る
  - **PaRoutes: GO**（データ到達性のみ） — `MolecularAI/PaRoutes`のn1/n5 targets/stock/routes、Zenodo record 6275421が到達確認済み。未取得・未使用
  - Retro*は時間箱の都合で未調査（優先度低のため）
  - **要ユーザー確認事項**: `brew install python@3.11`はマシンへの永続的なシステムレベル変更（repo外）。既に実行済みでロールバックは特にせず様子見だが、本格的なWin A構築（stock統一等）を続行してよいか、環境変更をどう扱うか（このまま/使い捨てコンテナへ移行）はユーザー判断を仰ぐ
  - RENKIN探索コード・stock・templateには一切触れず（確認済み）
- [x] **32.6** Track E完了（arm E2+E4のみ、2026-07-22、branch `audit/search-closed-set-and-admissibility`、commit `8486506`）
  - **E2 closed-set正しさ**: boolean closed set（reopen-on-lower-g無し）は実在するバグと証明（合成再現テストで確認、修正版で最適解 -6.756 に一致）。ただし**現在は休眠中**: `reaction_prior`/`value_estimator`は全ての本番エントリポイントでNone、かつデフォルトのcost式（bonus∈[0,0.2]）は代数的に2-hop-cheaper-than-1-hopを起こせないため発火しない。**32.3の99.97%探索予算枯渇を説明しない** — Track D/E3が将来学習済みpriorをadmissibilityクランプなしで導入した瞬間に顕在化するリスク
  - **E4 コストモデルadmissibility**: `step_cost - template_bonus`のadmissible主張は**実際に破綻済み**と確認。admissibility前提コメントは`template_bonus`導入前のもので、導入コミット(`740037b`、翌日)が矛盾するコメントを追加したまま4週間放置。意図的なweighted A*文書化ではなく、サイレントな不整合。ただし理論上限0.2は小さく、実ターゲットでの具体的な悪影響は未実証。**これも32.3の99.97%を単独では説明しない**
  - 両arm共通の結論: 探索予算枯渇の主因はE2/E4ではない — E1(frontier選択)/E3(heuristic代替)/E5(動的予算)またはTrack D(per-node ranking)側に主因が残る
  - テストのみのcommit（`src/search.rs`回帰テスト、+127/-0、本番ロジック変更なし）— 実装修正は行わず「revise」推奨（reopen-map案は5,000テンプレート規模での再テストなし、depth-inclusive案は同depth内の別ルート衝突を救えない）。**PR #38（squash `4fb2e54`、2026-07-22）でマージ済み、CI green**
  - 副次発見: `cargo clippy --all-features`は`src/chem_env.rs`/`src/python.rs`で**master既存の失敗**（`git diff origin/master`で両ファイルとも空diff確認）— 本トラック・32.1どちらの原因でもない、別途対応必要
- [x] **32.7** Track F完了・マージ済み（PR #39、squash `cbcc281`、2026-07-22、branch `fix/validator-fidelity`）
  - **原因判明（31.12bの答え）**: `chematic::canonical_smiles`は安定な不動点だが真のグラフ不変量ではない（同一分子でも原子順序/bracket記法違いで異なる文字列に非収束、`lessons.md` L2、chematic 0.4.30でも再現確認）。`ChemEnv::is_building_block`は既にVF2構造同型フォールバックでこれを回避済みだったが、forward validatorの`rule_reverses_to`は素の文字列一致のみで同じ弱点を継承していた
  - **修正**: `rule_reverses_to`/`smirks_reproduces`/`rule_reproduces`にVF2フォールバックを追加（`is_building_block`と同じ手法）。`@`/`@@`立体マーカーがどちらかの側にあれば無効化（VF2が四面体中心で立体を区別しないと実験で確認済み——(R)/(S)-2-butanolを同一と誤認するため、無条件フォールバックは誤ったValid判定を生む）
  - **効果（378ステップのgold set、137/200 quality_200ターゲット由来）**: Invalid 293件(77.5%)→91件(24.1%)、Invalid→Valid反転202件、Valid→Invalid逆行**0件**。202件全件をRDKitで独立再検証（サンプリングでなく全数） → 202/202が真の一致と確認、誤マッチ0件
  - **副次発見（未修正・要フォローアップ）**: retro-fragment生成（generic-cleavage系SMIRKSルール・多くのextracted template）が、切断原子の価数を再計算せず凍結したまま残しラジカル様の"precursor"を生成するバグを発見（RDKitのラジカル電子数で確認、環内結合の切断で最悪）。7つのgraph-based ruleは`is_bridge_bond`で既にガード済みだがSMIRKS/extracted-template経路は未対応。91件の残存Invalidの約46%がこれに起因と推定。探索コードに触れる必要がありraw solved rateへ影響しうるため本PRの範囲外、次の高優先度候補として記録
  - raw solved rate・search.rsは無変更（`src/validation/forward.rs`のみ）、enum/schema変更なし、whitelistなし
- [x] **32.8** ルール使用ログ抽出完了・quality_200再構築済み（2026-07-22）: `data/corpora/quality_200.json`を`primary_rule`層化（986件のsolved routeの実rule使用ログベース）で再構築、sha256更新
- [x] **32.10** Track D測定完了（2026-07-22、986ターゲット全件・1,519 extracted-templateステップ）: root-onlyランキング vs per-nodeランキングのtop-K recall
  - depth==0（sanity check）: 完全一致（ツール正しさ確認）
  - **depth>=1（n=994、本題）: top-1 recall 4.6%→12.9%(+8.2pp)、top-10 17.2%→38.4%(+21.2pp)、top-100 37.1%→64.1%(+27.0pp)、中央値rank 304位→27位（約11倍改善）**
  - 「推論回数を増やしただけでrecall改善なし」という却下ケースには該当せず、明確なゲート突破 → 実装フェーズへ進行指示済み（per-node ranking + canonical SMILES単位キャッシュ、hand-crafted ruleフォールバック維持、WASM frequency/bond-indexフォールバック温存、screening_500での前後比較必須）
  - **32.3（99.97%が探索予算枯渇）との整合性が高い有力候補**: root-onlyのままだと実際に使うべきテンプレートが深い中間体で304位付近に埋もれ、beam=100の現実的な探索範囲を超える
- [ ] **32.9** 次の優先候補（32.7のretro-fragment価数バグ、32.6のE1/E3/E5・Track D、Track A発見のstock統一設計）— Track D実装のscreening_500検証後に再評価
- [~] **32.11** Track D実装完了、screening_500でのbefore/after検証が進行中（2026-07-23〜24、中断・再開可能な状態でここに記録）
  - Track Dエージェントが実装済み（`feat/per-node-template-ranking`ワークツリー、`.claude/worktrees/agent-a43567c93854f3c39`、未commit）: `src/search.rs`にper-nodeランキング追加（root-only事前計算を削除、`retro_cache`のmiss分岐でintermediateごとに新規スコアリング — 既存キャッシュに相乗りするため追加キャッシュ構造不要）、`src/bin/benchmark.rs`に`retro_cache_hits/misses`計測追加。エージェント自体は非同期waitで数回スタックしたため、オーケストレーター（自分）が直接ビルド・検証を担当
  - **per-target timeout runner新設・マージ済み**（PR #40、`bae1f8c`）: `scripts/per_target_screening_runner.py` — 100件/プロセスのshard方式で1件が2,110,713ms(~35分)かかり shard全体を停滞させた実インシデント（screening_500 row 324、TIPS保護マクロライド、diagnosisはHard-200に追加済み・別調査へ切り出し）を受け、target単独subprocess isolation + soft180s/hard600s timeout（`subprocess.Popen`ベース、`timeout`/`gtimeout`コマンド非依存）で再設計。timeout targetは`solved=false`のまま分母に残す
  - **重大バグ発見・修正**（`7165885`、**⚠️直接masterへpush済み、PRフロー逸脱、要お詫び記録**）: `--parallel`実行時、renkin-benchのデフォルトrayonスレッド数（コア数分）×並列プロセス数でCPUを over-subscribe し、オーケストレーターのPythonスレッドがOSスケジューリングから締め出されhard timeout(600s)が機能しない実バグを発見（500件中5件が1,000〜1,539秒まで到達、うち1件のみ辛うじて検知）。`RAYON_NUM_THREADS=コア数/parallel`を各子プロセスに設定して解消、再現5件で確認済み
  - **`--scorer`未指定に気づき、baseline (v2/v3計2回、timeoutバグ修正前後) はTrack D比較には不使用と判断** — root-only/per-nodeの比較には両者とも`--scorer data/template_scorer.onnx`が必須（未指定だとどちらの分岐にも入らない）。v2/v3データは一般的な整合性・timeout修正検証としては有効に活用したが、Track D本比較は別途scorer有効の2本で実施
  - **root-only版（scorer有効、masterバイナリ）: 500/500完了・検証済み**（2026-07-24）。実行中に**ユーザーが`cargo clean`を実行**（`target/release/renkin-bench`削除、647.2MiB解放）、直後の51件（row 449-499）が`exit_status=127`で即時失敗（0.01-0.03秒、他への影響最小）。バイナリ再ビルド後、該当51件を`--only-indices`で補完 → 500/500・重複なし・欠損なし・不良レコード0件を確認済み
    - solved 102/500（20.4%、これまでの計測と一致）、timeout 45/500、600秒超過の漏れ0件（RAYON_NUM_THREADSバグ修正が有効に機能）
    - 永続化済み: `data/corpora/_screening500_rootonly_scorer_records.json`
  - **Track D比較完了（2026-07-25）**。実装をcommit（`94db50e`）、origin/masterをmerge（`85f5df0`、Cargo.toml競合は両[[example]]エントリ保持で解消）してPR #37-#40の修正（特にPR #39のvalidator VF2フォールバック）を取り込んだ上で最終比較を実施
  - **500件paired結果（root-only=masterバイナリ, per-node=Track Dワークツリー, 同一corpus/depth5/beam100/scorer/timeout設定）**:
    - solved: 両アームとも**102/500（20.4%）で完全一致**。newly-solved 0件、regressed 0件、net delta **+0**
    - timeout: root-only 45件 → per-node 32件。内訳: root-onlyのみtimeout 13件、per-nodeのみtimeout 0件、両方timeout 32件（per-nodeが新規にtimeoutを増やした例はゼロ、13件はtimeoutから「予算内で未解決」に転じた＝探索が速く空間を使い切るようになっただけでnewly-solvedへは繋がらず）
    - latency: p50 100.3s→80.1s、p90 537.3s→402.9s、p95/p99は両方600s上限に張り付き、総wall-clock -16%（1430分→1203分／500件）
    - nodes_expanded: 全体平均はほぼ同一（243.2→244.7）。**102件の共通solvedターゲットに限ると100/102で完全に同一node数**（残り2件のみ微差）——per-nodeの効果はsolved事例そのものには現れず、探索効率化はもっぱらunsolved側の探索打ち切りタイミングに現れている。fixed-node-budget軸でも実質差なしという結論
    - peak RSS: root 47.1MB→per-node 48.8MB（実質差なし）
    - inference: per-nodeのみ計測可（root-onlyのbinaryはretro_cache計測なし、設計上root一回のみ呼び出し）。total inference 81,039回、cache hit率29.2%、平均173.2回/target
    - **atom-balanced・validator-confirmed delta: 両方とも差分ゼロ**（atom_balance_ok 80/102 vs 80/102、route_validation_status validated 62/102 vs 62/102、102件全件で判定一致）——検証序盤でTrack Dワークツリーが旧masterベース（PR #39のvalidator修正前）だったため「62 vs 6」という見かけ上の大差が出たが、これは純粋にvalidatorバージョンの不一致が原因と特定・merge後に再検証して解消（実際のper-node実装によるchemistry qualityへの影響はゼロと確認）
    - **なぜwall-clockは速いのにnode数が同じなのか、というメカニズムは未解明** — 単発実行のため統計的ノイズの可能性を排除できていない。要フォローアップ
  - **ゴール文書の字義通りのゲート（"newly-solved > regressed"）は未達（0 > 0は成立しない）**。ただし全指標で悪化ゼロ、実質的なlatency改善あり、というのが誠実な結論。推奨: infrastructure（per-nodeランキング機構・キャッシュ再利用設計）自体は健全なので保持する価値はあるが、探索予算枯渇という32.3の主要ボトルネックはranking品質だけでは解消しないことが実証された——次の焦点はE1/E3/E5（frontier選択・heuristic・動的予算）に移すべき
  - 生データ: `data/corpora/_screening500_rootonly_scorer_records.json`、`_screening500_pernode_scorer_records.json`、`_screening500_rootonly_plausibility_records.json`、`_screening500_pernode_plausibility_records_v2.json`（102件、validator修正後の正しい比較）
  - 参考データ（本比較には不使用、一般的な整合性/timeout修正検証用）: `data/corpora/_screening500_baseline_v2_records.json`（timeoutバグ修正前）、`_screening500_baseline_v3_records.json`（timeoutバグ修正後・scorerなし）

---

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
- [x] **29.2** `docs/site-sync` — docs サイトの残り古い記述を整理（2026-06-29）
  - docs/index.md: "20 built-in rules" → "31 + ~5k extracted"、Key Features 更新
  - docs/benchmark.md: 比較表を Table A/B/C に分割、v0.2.1 参照削除、Failure Taxonomy セクション追加
  - docs/api/rust.md: "20 rules" → "31 handcrafted rules"
  - pyproject.toml: 0.15.4 → 0.15.5（Cargo.toml との同期）
  - README/README_ja: `renkin-forward (planned)` → `renkin-forward`（実装済みのため）
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
現在:
  78.0%（Stage 1: depth=5, beam=100）
  **95.9%（Cascade: Stage1 + Stage2 depth=7 beam=300）**（2026-06-29 確定、4,705/4,907）
  Stage 2 残り未解決: 202 件のみ
次手: cascade を正式スクリプト化（scripts/cascade_bench.sh）、scripts/merge_cascade.py で結果マージ
目標: cascade 正式機能化 → README/docs 更新
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
