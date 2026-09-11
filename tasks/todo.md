# RENKIN - Todo

## Current objective

Phase 53を現行目標とする。native探索成功を保護しながら追加候補を段階投入し、候補生成・
template被覆・stock差を順に改善する。同一条件のAiZynthFinder成功率超えを目標とし、
既存成功regression=0、invalid=0、再現可能な測定manifestを必須条件とする。

## Phase 53: Candidate augmentation without regression

- [x] **53.1** native探索成功を保護し、未解決かつ正常終了した対象だけへ直接生成候補を段階投入する`find_routes_with_retro_generator_retry`を追加。既存成功を追加候補で置換しない構造を固定し、search回帰・clippyを通過
- [x] **53.2** AiZynthFinder-only gap 28件で500/5,000 template候補poolを比較。候補数798→2,010、zero-candidate 0/28を確認。単純なtemplate増加はbeam crowd-outを招き得るため、常時混在ではなく段階投入を採用
- [x] **53.3** RENKINの5,000-template poolをdirect-generator fixtureへ変換して追加候補を測定。gap 28件では3/28のままでroute回収なし、既存成功regression=0、invalid=0、timeout/crash=0。候補を増やすだけではdownstream到達性を改善しないnegative resultを記録
- [x] **53.4** common-stock unionでstock差を分離測定。500-template baseline 3/28に対し5,000-template coverage recoveryは5/28（+2件、+7.1pp）、regression=0、invalid=0、timeout/crash=0。直接候補追加は3/28で変化なし
- [x] **53.4b** staged retryのbeam crowd-out境界を強化。native-only上位beamを先に保持し、direct-generator候補には明示的な追加枠だけを割り当てる`retro_generator_slots`を追加。既定値0、native successはgenerator armを実行しない回帰テストを追加。再ビルド済みbinaryでgap 28件を再測定し、3/28、regression=0、invalid=0、timeout/crash=0を確認。現fixtureの候補はdownstream routeを増やさず、candidate admission自体は安全だがcoverage gainなし
- [x] **53.5a** 500→5,000→9,997 usable templateのfull staged recoveryを同じgap 28件・common-stock unionで測定。5,000 tierの5/28から7/28へL1399/L371を追加回収し、500 baselineの3件は全て維持（regression=0）。element-accountingでL274、beam wideningでL370も回収。invalid=0、timeout/crash=0だが、p95 elapsed 54.67秒、p95 RSS 353.3MiBとなるため常時既定化せずopt-inのまま維持
- [x] **53.4c** 現行の全テンプレート入力（CLI実測9,974 rules）からgap 28件の直接生成候補を再生成し、native beam保持付き段階投入を再測定。3/28のままで追加回収なし、baseline回帰0、invalid=0、timeout/crash=0。候補poolは2,676行、zero-candidate 0/28、生成p95 0.025秒/targetで、欠落ではなく候補の化学妥当性または後続stock到達性が主因と判断
- [x] **53.5b** 9,974-template候補poolをcommon-stock unionへexact canonical-SMILES照合する`diagnose_candidate_stock_frontier.py`を追加。2,676候補のうち全前駆体がstockに入るものは3件・3/28対象のみで、残り25対象はdownstream searchが必須と分解。構造・stockの判定を混同せず、診断JSONを保存
- [x] **53.5c** staged direct-generatorの追加容量を`--retro-generator-slots N`で明示指定可能にした。未指定時は0、上限1,000でfail-closed。native beam保持と投入容量を比較manifestへ固定でき、CLI入力検証テスト・clippyを通過
- [x] **53.5d** direct-generator候補の入口でroute-integrityを事前検証し、構造的にstrict routeになれない候補をfrontierへ入れないようにした。native候補には適用せず、invalid候補の除外テストと既存CLI回帰テストを通過
- [x] **53.5e** static generator変換時にpoolの`best_upstream_rank`を`source_rank`へ保持するよう修正し、最新release binaryでgap 28件を再測定。結果は3/28のまま（regression=0、invalid=0、timeout/crash=0）で、順位復元だけでは追加回収なし。candidate selectionの欠陥を修正したうえで、次はdownstream展開側を改善する
- [x] **53.5f** root候補から抽出した2,617中間体へ9,974-template候補を供給し、stockヒット優先の各32候補にbounded selectionしてroot＋intermediate fixtureを構成。gap 28件の段階投入は3/28のまま、regression=0、invalid=0、timeout/crash=0。p95 RSSは約155MiBで、候補供給範囲の拡大だけではpositive recallを増やせないことを確認
- [x] **53.5g** 中間体候補をstockヒット数＋template順位で選択し、direct-generatorの`source_rank`をordering-only補正へ接続。最新release binaryでgap 28件を再測定し、3/28→5/28（L1399/L370を追加回収）、baseline 3件を全保持、regression=0、strict invalid=0、timeout/crash=0を確認。p95 RSSは約161MiB、L370の`stereo_center_count_mismatch`/`charge_imbalance`は情報警告として記録
- [x] **53.5h** 中間体候補上限48をpairedに測定。strict coverageは4/28に低下し、32候補の5/28を再現できず、p95 RSSも約208MiBへ増加。regression=0、invalid=0、timeout/crash=0だが、候補増量による負荷・順序悪化として不採用。stock優先32候補を現行bounded sweet spotとする
- [x] **53.5i** 比較ハーネスへ`--retro-generator-slots`を接続し、実binaryへの伝播とconfiguration_id（例: `...retro_generator-s20`）を1件smokeで確認。追加枠条件をmanifest単位で再現可能にした
- [x] **53.5j** direct-generator候補のstock充足率を検索コストへ混ぜる案をpaired測定。native armには影響しないが、gap 28件で従来5/28から4/28へ悪化したため撤回。stockは選択・診断層に限定し、候補の化学コストと混ぜない負の結果を記録
- [x] **53.6** `bond_index`が空、またはindexed rule適用後に候補ゼロとなった場合だけ全rule setへcoverage fallbackする経路を追加。非空で候補が得られるindexed検索の集合・順位は不変で、両fallback回数をdiagnosticsへ記録。empty-index fixture、clippy、formatを通過
- [x] **53.6a** 比較ハーネスへ`--bond-index`とfallback診断値を接続。fixed gap 8件でroute foundは1/8、empty/no-proposal fallbackはいずれも0件、invalid=0、timeout/crash=0。既存の非indexed armのconfiguration IDは不変
- [x] **53.6b** staged direct-generator（中間体stock優先32候補・slot20）と`bond_index`を同時にfixed gap 28件で測定。strict coverageは5/28で従来最良を維持、baseline回帰0、invalid=0、timeout/crash=0、両fallback発生0、p95 elapsed約4.84秒、p95 RSS約155MiB。bond-index併用による追加回収はなかった
- [x] **53.6c** bond-indexが1件以下の候補しか返さない場合に限り全rule setを再照合し、indexed結果へ追加してcandidate-level mergeへ渡すlow-recall fallbackを実装。fixed gap 28件（direct-generator slot20併用）では5/28を維持、regression=0、invalid=0、timeout/crash=0、p95 elapsed約4.55秒、p95 RSS約152MiB。改善は未確認だが、既存のbond-index armを悪化させないことを確認
- [x] **53.6d** direct-generator追加枠内で即時stock終端候補を強制予約する案をpaired測定。coverageは5/28のまま、p95 elapsedは約5.16秒へ悪化したため撤回。追加枠はsource-rank順を維持し、stockは候補選択・診断層で扱う
- [x] **53.5k** 全候補fixtureに対する1段stock-lookahead選択器を追加。候補自身のstockヒット、artifact内の次段stock終端可能性、source rankで各targetを最大32件へbounded selectionする。fixed gap28では4/28となり既存の5/28から悪化したため検索fixtureには採用せず、候補選択の負の結果と再現用テストのみ保持
- [x] **53.7** recovery modeのbaselineをgeneratorなしのnative passへ固定し、native失敗後だけdirect-generatorを追加枠付きで実行する段階を追加。recoveryでも既存native候補をgeneratorが押し出さない構造に変更し、recovery関連テスト・clippy・release buildを通過
- [x] **53.8** fixed gap 28でnative baseline→direct-generator→depth/beam→5,000/9,974-template coverage tierの統合cascadeを測定。7/28を回収（baseline 3、generator 2、element-accounting 1、coverage 1）、baseline成功のregression=0、invalid=0、timeout/crash=0、p95 elapsed約26.6秒、p95 RSS約408MiB。coverage改善は確認できたが、常時defaultではなく明示的deep recovery候補とする
- [x] **53.9** 明示的な`retro_generator_slots`指定時のみ、初回generator miss後に追加枠を最大2倍へ広げる`retro_generator_widened` recovery stageを追加。native beamを両段階で先に保持し、fixtureで段階遷移と既存候補非置換を検証。defaultのgeneratorなし挙動と、slots=0の単一段階挙動は維持
- [x] **53.10** 全中間体候補artifact（約112MiB）を評価できる専用128MiB入力上限を追加し、fixed gap先頭8件を測定。coverageは2/8でstock32 fixtureから増えず、p95 RSSは約606MiBへ増加したため、全artifact投入は研究用diagnosticに限定
- [x] **53.5l** AiZynthFinderのgap route edgeとRENKIN候補poolをcanonical identityで照合し、残存gapの正解一段候補は21/21でpool内にある一方、代表L1006の競合経路は二段目以降の保護基段階で欠落することを確認。既存の方向を変更せず、neutralな1/2級aliphatic amineからN-Boc precursorを生成する限定的な`boc_protection_retro`を追加した。L1006では候補生成段階は通過したがstrict routeは未回収のため、未解決53.5の下流到達性調査は継続
- [x] **53.5m** 競合route edgeをRENKINのcanonical candidate poolとexact照合する`diagnose_competitor_route_coverage.py`を追加。固定gap28ではAiZynthFinderの143 edge中、poolにtarget候補あり68、precursor multiset完全一致53、target欠落75を再現可能なJSON診断として出力し、候補欠落と下流到達性を分離できるようにした。さらに欠落75 targetをfull 9,974-templateで直接生成すると75/75がnon-zero（3,928 rows、p95 0.0108秒/target）だったため、現時点の主因を「template適用不能」とは扱わず、中間体投入・選択・下流stock到達性の問題として切り分けた。固定cohort用テストを追加
- [x] **53.5n** `expand_candidate_frontier.py`を追加。既存poolを削らず、前段で得たprecursorを次段の`renkin-pool-gen`へbounded投入し、全roundの候補をunionする。compiled stock指定時はstock-terminal moleculeを次段入力から除外し、round別件数とartifact hashを記録する。外部モデル・labels・競合routeを使わない候補生成基盤とテストを追加
- [x] **53.5o** frontier投入上限32と256を同一gap28・同一recovery条件でpaired測定。いずれも7/28で既存成功のregression=0、invalid=0、timeout/crash=0。256は候補rowsを27,541まで増やしても新規成功なし、p95 total elapsedは約95.0秒（32は約87.1秒）へ悪化したため、候補数の単純増量は不採用。次は中間体選択のpositive recallと下流stock到達性を分離して改善する
- [x] **53.5p** frontierの次段targetを親候補のstock hit ratio・hit数・source rankで選ぶstock-aware policyを追加し、固定gap28・32 target・2 roundでpaired測定。候補unionは5,333 rows、成功率は7/28で変化せず、regression=0、invalid=0、timeout/crash=0。p95 total elapsedは約102.0秒となり、stock近接度だけの選択は不採用。次は候補グラフの下流到達性を明示的に評価する
- [x] **53.5q** full candidate poolのtarget→child candidate graphを使い、child count・child stock-terminal count・child stock ratioを選択に加える`select_retro_generator_graph.py`を追加。root targetは全候補を保持し、中間targetだけbounded selectionする非破壊selectorとテストを追加。固定gap28の同一recovery条件で7/28→9/28（新規L274/L370）、既存成功regression=0、invalid=0、timeout/crash=0、route tree parseable=9/9を確認した。p95 total elapsedは約101.5秒で、速度コストを記録した
- [x] **53.5s** graph selectorをfull VAL-200へ適用。same-run native first attempt 133/200からrecovery result 143/200へ10件追加回収し、completed native successのregression=0、strict validated=143/200、parseable=143/143、crash=0を確認。timeout=5/200は成功に含めず、条件差を明示した比較reportを保存。AiZynthFinderの123/200は参考値であり、same-condition superiority claimは保留
- [ ] **53.5t** 同一stock・同一120秒budgetでAiZynthFinderを再測定する準備を確認。既存のshared-stock HDF5/configは存在するが、Docker daemonが応答せず実行開始不可。RENKIN側の失敗ではないため、環境復旧後に同一条件paired gateを再開する
- [x] **53.5r** graph selectorで新規回収したL274/L370を同一artifact・同一recovery条件で再実行し、2/2 route_found、strict validated 2/2、route tree parseable 2/2、invalid=0、timeout/crash=0を再現。新規回収の一過性ではないことを確認した
- [x] **53.5u** graph selectorがJSONL poolとstatic generator artifactの両形式を直接扱えるようにし、artifactのprovenanceを保持したまま中間targetだけをbounded selectionする入出力境界を統一。root候補非置換とformat変換の4テストを追加
- [x] **53.5v** VAL-200の競合route edgeから候補欠落targetを310件抽出する`--missing-targets-output`を追加し、full 9,974-templateで310/310 targetに候補を生成（20,808 rows、zero-candidate=0）。既存poolへunion後、edge target被覆は68/378→378/378、exact precursor multiset一致は52/378→265/378へ改善。非置換staged recoveryでregression=0を確認するまでdefault投入は保留
- [x] **53.5w** 欠落target候補を既存poolへunionしたVAL-200非置換recoveryを完了。native first attempt 133/200、最終143/200、追加10件、regression=0、strict validated=143/200、parseable=143/143、timeout=5、crash=0。既存graph selectorの143/200から増加なしのため、候補生成ではなく下流selection・stock到達性の改善へ移行
- [x] **53.5x** `verify_non_displacing_recovery.py`を追加し、recovery attemptsのnative first successとfinal selected resultを行単位で比較する回帰ゲートを固定。candidate expansion VAL-200へ適用し、baseline=133、final=143、recovered=10、regression=0、timeout=5、crash=0を機械検証
- [x] **53.5y** candidate-graph selectorへopt-inのbounded multi-hop stock lookaheadを追加し、固定gap28で8/28、regression=0、invalid=0、timeout/crash=0を測定。既存graph selectorの9/28を下回ったためdefault化せず、stock到達性を順位へ直接混ぜる方針の負の結果を記録
- [x] **53.5z** candidate-graph selectorへopt-inの多様性選択を追加。`(precursor count, source-rank band)`のbucketから先に候補を保持し、root候補は全保持するlabel-free proxyを6テストで検証。実artifactでroot 134 targetを保持、中間2,664 targetを各16候補へbounded selection。fixed gap28で7/28（baseline 5/28、追加2件）、regression=0、invalid=0、timeout/crash=0を確認したが、既存graph selectorの9/28を下回ったためdefault化しない
- [x] **53.5aa** 候補union後の113件のexact precursor mismatchを、候補poolだけで再現可能な`diagnose_candidate_mismatch.py`へ分解。partial overlapは42件、共通precursorがないno overlapは71件。同一canonical stockを使った保守的なgraph到達性では49件がstock到達可能（partial 24、no-overlap 25）、64件が到達不能。さらにpartial overlapの欠落precursorはstock終端が28件、下流探索が必要なものが14件だった。候補数を増やす前に、正規化・stock identity・反応family・下流到達性を別々に診断する根拠を固定した（化学的同値性は未判定）
- [x] **53.5ab** stock到達可能なpartial overlap 24件を中間target保護対象として無制限保持し、固定gap28でpaired測定。baseline 5/28、final 7/28、追加2件、regression=0、invalid=0、timeout/crash=0。既存graph selectorの9/28を超えず、p95 90.24秒のため本番policyには不採用。次は24件をreaction family・representation別に分類する
- [ ] **53.5ac** partial overlap 24件を反応family・正規化表現・下流stock到達性へ分解し、候補数や無制限保護ではなく、family別の非押し出し補完をpaired評価する。候補unionでtarget-level coverageは378/378、exact precursor matchは265/378であり、候補数単純増加や順位proxyは既に不採用
- [x] **53.5ac-prep** `diagnose_partial_overlap.py`を追加。partial overlapをRDKit canonical/stereo-stripped表現差、透明な`reaction_family_proxy`、欠落前駆体のdirect-stock/downstream-or-unavailableへ分解する。これはroute validityや検索順位を変更しないheuristic diagnosticで、入力hashと非断定的evidence levelを保存する
- [x] **53.5ac-diagnostic** 実VAL-200 coverage atlasとcandidate expansion artifactを同一cohortで再計算し、partial overlap 42件を検出（amide 9、ester 2、sulfonamide 4、carbonyl-oxygen 4、other/unknown 23）。表現判定はcanonical 3、stereo-only 2、not-equivalent 32、unparseable/非同値8、stock direct 1、downstream-or-unavailable 44。24件のstock-reachable subsetはamide 6、carbonyl-oxygen 1、ester 1、sulfonamide 2、other/unknown 14に分解できた。結果は`partial_overlap_diagnostic_recomputed.json`へ保存した。これは拡張artifactのheuristic診断であり、family別非押し出しpaired評価やroute成功の証明ではない
- [x] **53.5ac-baseline** 24 edge（21 unique target）をfamily別sampleへ分割し、同一shared-stock/recovery設定でbaseline stratificationを実行。amide 4/5、carbonyl-oxygen 1/1、ester 1/1、other/unknown 11/12、sulfonamide 2/2。全21 targetでinvalid・timeout・crash=0。direct-generatorは中間target測定ではbaseline stageのみ発動したため、これはsupplementationのpaired結果ではなく、family別baseline証跡に限定する
- [x] **53.5ac-recovery-audit** 同じ21 unique targetを同一stock・template・depth/beam・30秒cascade予算でRecovery監査。全21件が正常完了し、route_foundは3/21、generatorは19/21件で起動、5,043 proposal中4,980件を追加枠へadmit、integrity拒否63、parse/自己同一拒否0。generator stage単独の新規回収は0件で、回収3件はbaseline/beam/depthの後段。これはgenerator候補の非押し出し監査と下流ボトルネックの証拠であり、標準modeの19/21 baselineとの差を同一実行内で比較したpaired efficacyではないため、53.5ac本体は未完了のまま維持する
- [x] **53.5ad** direct-generatorの供給・採用・integrity拒否・parse拒否・自己同一候補拒否を`SearchStats::crowd_out`へ記録する計装を追加。固定gap28へ適用し、generator stage 23,685 proposals中23,407 admitted、278 integrity rejected、parse rejected 0を確認。最終7/28で、候補入口ではなく下流探索・stock終端が主因と切り分けた。`cargo check`・focused Rust tests・formatを通過
- [x] **53.5ae** generator候補をelement/beam/depth retryへ引き継ぐことで後続budgetでも再利用可能にした。固定gap28でbaseline 5/28、final 7/28、追加2件、regression=0、invalid=0、timeout/crash=0を確認したが、既存graph selectorの9/28を超えず、p95 102.34秒へ悪化。opt-in実験として保持しdefault化しない
- [x] **53.5af** full recovery policyでgenerator候補を後段のelement-accounting retryまで段階投入。固定gap28でbaseline 5/28、final 9/28、追加4件、regression=0、invalid=0、timeout/crash=0、p95 76.08秒。既存graph selectorの9/28と同率で、AiZynthFinder超えの根拠にはならない。結果は`data/comparison/formal_v1.0.3_candidate_20260909/full_policy_gap28/`へ保存し、default化せずopt-in診断として維持
- [x] **53.5ag** VAL-200を同一shared-stock条件で再測定。RENKINはroute_found 134/200（67.0%）、validator-confirmed 126/200（63.0%）、timeout/crash 0。AiZynthFinder保存値のroute_found 134/200と件数同率だが、同一セッションのpaired再実行ではないため優位性は未証明。証跡は`data/comparison/formal_v1.0.3_candidate_20260909/formal_200_rerun_20260909/`

## Phase 52: Audit-native agent bridge and policy-aware stock（次期候補）

- [x] **52.1 / O6.1** versioned `AuditReceipt`と親子MCP execution traceを実装。modern `tools/call`の`_meta.io.renkin/auditReceipt`にtool/version/model、argument/result hash、task ID、parent task ID、timestamp、status、failure codeを出力し、機密本文は保存しない。legacy wire shapeは維持。receipt IDとcontent hashは決定論的、timestampは実行時情報として分離し、単体・MCP回帰・workspace全体テストを通過
- [x] **52.2 / O6.2** private/constrained stock policy engineを追加。`accepted/rejected/unknown`と決定論的reason code、policy hash、vendor/price/lead-time/region/banlistを監査する。vendor tableのregion列、allow/block region、hazard、availability、価格・納期制約をfail-closedで評価し、region回帰テストを通過
- [x] **52.3 / O6.3** Bridge adapter loss reportを追加。canonical interchangeにfield単位の`preserved/normalized/inferred/dropped/unsupported`を必須化し、schema/空fieldをstrict importでfail-closed検証。conditions未対応やreaction evidence欠落をunsupportedとして成功routeとは分離
- [x] **52.4 / O6.4** canonical route exportとloss-report必須のstrict importerを実装。schema version、round-trip、loss accounting、strict audit validationをテストで固定し、競合assetの変換・再配布は行わない
- [x] **52.5 / O6.5** retro → condition → forwardのagent replay gate、MCP adversarial、secret redaction、receipt hash再現性を完了。replay kernelでreceipt integrity、3 phase completeness、failure continuation、manifest hash、redaction分類を検証し、実プロセスMCP transcript、workspace全体、security gateを通過

## Phase 51: Search profile manifest contract（次期候補、2026-09-08）

- [x] **51.1 / O5.1** named `fast`/`balanced`/`deep` profileの実効baseline・recovery budget、timeout、diversity、stage policyをschema-versioned `search_profile` JSON metadataへ記録。profile未指定時のlegacy JSON shapeは維持
- [x] **51.2** `balanced` CLI smokeでdepth=5/beam=100からdepth=6/beam=200、15秒、native policyのmetadataを確認し、`cargo test -p renkin`を完走
- [x] **51.3** 比較harnessへnamed profileの転送、configuration IDへのprofile識別子、per-target `search_profile` metadata保持を追加。adapterテスト17件を通過
- [x] **51.3b** completed rowのprofile metadataをfail-closed検証し、aggregate/manifestへprofile identityとschema versionを記録
- [x] **51.3c** `scripts/compare_search_profiles.py`で同一cohortの3 profile実行、arm artifact hash、portfolio manifest生成を自動化。dry-runと単体テストを追加
- [x] **51.3d** profile smokeで発見したaggregateのconfiguration ID未束縛を修正。同一10-targetで3 arm完走、rows/aggregate/manifest/portfolio hashを確認
- [x] **51.3e** 同一10-targetでfast/balanced/deep smokeを実測。各1/10 route_found、invalid=0、timeout=0、crash=0。p95 total elapsedはfast約6.1秒、balanced約15.0秒、deep約120.1秒で、profile metadata・arm hash・portfolio manifestの生成を確認。正式VAL-200のpaired gateは未完了
- [x] **51.4** 同一VAL-200 cohortでfast/balanced/deepをpaired測定。route_found/strict validatedはそれぞれ24/200 (12.0%)、27/200 (13.5%)、29/200 (14.5%)、invalid・timeout・crash=0。p95 total elapsedは8.085秒、15.073秒、86.456秒、p95 RSSは46.3、46.5、79.1 MiBで、profile・arm・portfolio hashをmanifestから再生成できることを確認した。証跡は`data/comparison/search_profiles_formal_20260911_retry/`。

## Phase 50: Search hot-path acceleration（次期候補、2026-09-08）

- [x] **50.1** macOS sampling profileで既定SA heuristicのring perceptionがmain-thread sampleの約60%を占めることを特定
- [x] **50.2** native・untimed・default estimator・元素制約なしの検索で、未評価かつ非stockのprecursor SA scoreをRayonで先行計算。既存と同じexact score/cache/frontier orderingを維持し、timeout/custom estimator/WASMは従来経路を維持
- [x] **50.3 / rejected experiment** simple exact `H`/`Hn` prepared prefilterは候補等価性gateを通過したが、最終5回平均373.38msでSA並列化単独349.96msを安定して上回れず撤回
- [x] **50.4** 500-template×10-targetのprepared/unprepared候補等価性を確認。固定10-target smokeは直後5回平均349.96ms、後刻の高負荷下7回平均393.72ms（中央値381.31ms）で、直前663.82ms比40.69〜47.28%短縮。solved/nodes/cache/raw/dedup accounting不変
- [x] **50.5** 固定VAL-200・185,042件union stock・5,000 templates・depth 5・beam 100で共有プロセスfull-searchを完走。RENKINのstrict stock-terminal成功は128/200、固定AiZynthFinder行は123/200（paired差+2.5pp、95% CI -4.5〜+9.5pp、McNemar p=0.576）。点推定は上回ったが統計的優位とはしない
- [x] **50.6** SA先行計算を同一40-target full-searchでON/OFF測定。合計105.77s対328.07s（67.8%短縮）、40/40で高速、route hash・nodes・候補数は完全一致
- [x] **50.7** 比較aggregateへ「共通validator合格か、stock確認済みdepth=0」のstrict stock-terminal指標を追加。batch runnerへaggregate/manifest出力を追加し、zero-stepを非評価反応として保持しつつ正当な直接購入成功として明示集計
- [x] **50.8** file-backed model adapterのnative-only loader/importをcall-site単位でcfgし、`wasm-pack build --target nodejs --no-default-features`とNode quickstartを通過
- [x] **50.9** childごとに全frontierを再評価していたdefault heuristicを、親から保持されるprefix 1回と追加precursorの差分foldへ変更。同一40-targetで99.86s→74.09sへ至る前段として105.77s→99.86s（5.6%短縮）、37/40高速、route hash・nodes・候補数不変
- [x] **50.10** 生成precursorがstock policyで標準化済みという不変条件をhot pathへ反映。外部rootだけをparsed moleculeから事前解決し、生成済みcanonical SMILESのstock missでparse/standardize/canonicalizeを再実行しない。固定40-targetは99.86s→74.09s（25.8%短縮）、40/40高速、p50 1.219s→0.980s、p95 8.564s→5.983s、24 routeのhash・nodes・候補数不変
- [x] **50.11** direct generator precursorも同じ標準化・molecule intern経路へ統合し、設定時にfrontier molecule cacheが欠ける潜在panicを除去
- [x] **50.12 / rejected experiments** `PathNode.target`の`Arc<str>`化は固定40件で0.14%悪化、negative stock missを保存しない案は3.39%悪化したため双方撤回
- [ ] **50.13 / publication boundary** 同一per-target timeout・独立RSSを持つ競合latency再測定は別gate。共有プロセスの探索時間からAiZynthFinderへの正式速度優位を主張しない
- [x] **50.14 / rejected experiments** 既定SAの`[1.0, 1.5]`上下界でbeam圏外を事前除外する案は固定40件で74.09s→87.61s（初対象では最終eviction 975件中28件だけ事前除外）、target間共有SA cacheは77.68sで、いずれも改善を示さず撤回。route hash・nodes・候補数は全件一致

## Phase 49: Open-issue reconciliation（優先度監査・local corpus修復、2026-09-06）

- [x] **49.1** open issueをcore correctness、search quality、再現性、外部data依存の順で再棚卸し。既存dirty差分とstaged状態を先に監査し、他作業の変更を保持
- [x] **49.2 / Issues #98 / #100** revision不明だったoptional local 5,000-template corpusを、固定USPTO-50K TRAIN revision `08a575f0546b2be57242997fd45f684d6814d5a9`から再生成。全5,000件load/concrete application PASS、SHA-256 manifestと環境情報を記録し、旧bytesはhash付きgitignored backupへ退避
- [x] **49.3** 再生成corpusをrelease binaryの`doctor templates`で検証。5,000 logical rules、direct 1,538、hash-atom expansion 3,462、unsupported/rejected 0を確認
- [x] **49.4 / Issue #77** 除去済み`aryl_amine_retro`の未特定原因を分離。bare mapped-atom RHSへのsubstituent carry-throughがfused ringを重複走査し、N-bearing raw productをsplit前からopen aromatic chainへ壊すことを固定fixtureで確認。parser緩和や旧SMIRKS再有効化はせず、明示graph partitionを設計できるまで永久除去を維持
- [x] **49.5 / Issue #101** 既存成果をissueのcompletion boundaryへ照合。beam 100/200/300感度測定、crowd-out diagnostics、ordering reranker、diversity reservation、score-first retry、staged recoveryまで実装・測定済み。単純なdefault beam増加と常時diversityは非単調または回帰のため却下し、既定`Off`を維持
- [x] **49.6 / Remaining priority** 固定disjoint-200 cohortを新しい独立評価cohortとして採用し、初回と同条件rerunを完了。両runともroute_found/strict validated 21/200 (10.5%)、invalid・timeout・crash=0で、input/stock/template hashとconfiguration IDが一致した。このcohortに限る再現性・一般化を確認した。外部大規模corpus（#86）と正式forward baseline（#61）は別境界として保留、#72は49.8の安全境界で完了扱い
- [x] **49.6b** disjoint-200 fast rerunをhash突合。初回p95 total elapsed 5.701秒、再実行5.390秒、初回p95 RSS 44.7 MiB、再実行44.3 MiB。速度差は実行揺らぎとして記録し、route countとfailure rateの一致を再現性判定に使用
- [x] **49.6a** 固定disjoint-200 cohortへ現行fast profileを初回適用。route_found/strict validated 21/200 (10.5%)、p95 total elapsed 5.701秒、p95 RSS 44.7 MiB、invalid・timeout・crash=0。入力・stock・template hash付きmanifestを保存したが、再実行による再現性確認前のため49.6本体と競合優位性の判定は保留
- [x] **49.7 / Issue #61** forward benchmarkの`train-extracted`を、`--template-manifest`必須のfail-closed modeとして解禁。template/corpus SHA-256、split protocol version、`included_split: "train"`を実入力へ照合し、missing/malformed/stale/wrong-mode manifestをrule load前に拒否。これは抽出手続き自体の独立証明ではないため、正式runでは抽出commandとdataset provenanceを別途保持
- [x] **49.8 / Issue #72 boundary audit** 既知500-template用ring-context sidecarはtemplate corpusと共にpackage対象外で、custom corpusへ安全に一般化できないことを再確認。既定自動適用はlicense/provenance/互換性境界を越えるため却下し、opt-inを維持。完成routeでは既存のatom-level aromaticity・directional element accounting境界が有効
- [x] **49.9 / Issue #240 reproducibility** rdchiralの立体template生成がPython標準`random`ではなく`numpy.random.shuffle`を使うことを特定し、reaction単位で両RNGをSHA-256 seed固定・caller state復元。固定TRAIN 40,008件の独立2回抽出が全1,359 templateでbyte-identical（SHA-256 `9b868754...a47a9d4`）。`count>=25`の182 rulesと既存union hashは不変のためVAL/holdout再測定不要
- [x] **49.10 / Issue #240 sparse selection** TRAIN由来radius-zero rulesをdisjoint VALで決定論的に選ぶbounded selectorを追加。4 rulesでVAL positive-present 165→170、候補17,146→17,190（1.0026x）、baseline loss/parse failure 0。ただし凍結した残18件では新規route 0、timeout/crash 0、完成候補棄却10,433件は全て`unaccounted_target_element`。不採用とし同holdoutで再調整しない
- [x] **49.11 / CLI safety** `renkin-pool-gen --help`が未知引数を無視して既定4,903件runを開始する欠陥を修正。help表示、unknown/duplicate/missing-valueのfail-closed検査と単体testを追加
- [x] **49.12 / #44.8 preflight** 比較manifestへ`Cargo.toml`/`Cargo.lock`のhashとcrate/chematic versionを記録し、依存更新をまたぐ古い正式測定値の混同を防止。現環境で`renkin 1.0.1`/`chematic 1.0.7`の捕捉を確認。現ソースのRust再ビルド後に固定VAL総合再測定を実施する
- [x] **49.13 / #44.8 resume guard** 既存manifestで再開する前に、記録済みの全入力集合と現在のSHA-256を比較するfail-closed検証を追加。Cargo manifest/lock、stock、template、sidecar、reranker、coverage tierの差し替えを測定途中で混在させない
- [x] **49.14 / #236 diagnostic refinement** 比較validatorへ`target_element_excess_counts`を追加し、`unaccounted_target_element`の元素別不足量を決定論的に記録。判定・検索・benchmark denominatorは変更せず、独立cohort到着後の原因比較に利用可能
- [x] **49.15 / artifact replayability** 比較行へ正規化済みtarget/precursorのみの`route_edge_snapshot`を追加。tool-native outputやscoreを保存せず、再測定なしの構造・元素診断を可能にする

## Phase 48: AiZynthFinder-only failure reduction（staged recovery実装・58件再測定完了、2026-09-05）

- [x] **48.1 / Issue #236** 正式v1.0.1比較のAiZynthFinder-only 58件を再分類。全件がdepth/beam上限へ到達し、11件では完成候補828経路が構造整合性境界で拒否され、その全件に`unaccounted_target_element`を確認
- [x] **48.2** omission allowlistで不正経路を通す案をA/B後に却下。増加した5経路は共通validatorで全て`unaccounted_target_element`となり、成功率のために正式品質ゲートを緩めない境界を維持
- [x] **48.3** 現行のstrict `gated`を58件へ適用し、`L3738`・`L348`・`L4863`の3経路をstructure・stock・directional element accountingで独立再検証。既存591成功例も590 accounted + 正当なzero-step `L1394`の全件を維持し、実回帰0件を確認
- [x] **48.4** 常時gatedの候補ごと再parse除去を2方式で試作・同一58件paired測定し、明確な高速化が得られなかったため両方撤回。未実証の複雑化をproductionへ残さない
- [x] **48.5** `retry-on-integrity-failure`を追加。通常の`Off`検索で有効経路がなく、完成経路が`unaccounted_target_element`で実際に拒否された場合だけ、同じprepared templatesを再利用してstrict `Gated`検索を行い、発火理由と回復有無をJSONへ記録
- [x] **48.6** 比較adapter/runnerへ同policyと一意なconfiguration IDを配線。正式58件では11件だけ再探索し、`L3738`・`L348`の2件をaccounted/stock-terminal経路として回復。`L4863`は初回に具体的整合性拒否がないため低コストretryの対象外として維持
- [x] **48.7** CLI fixtureで「不正高優先候補によるbeam crowd-out→整合性拒否→strict retryで正当経路回復」と、zero-step成功時のretry非発火を固定。比較ハーネス42件・元素収支CLI 7件を通過
- [x] **48.8 / P1 / #238** 修正版diversity-reserved beam（20/100枠）を固定58件で再評価。`Off`のvalid 0件に対し`Active`は`L3738`・`L87`・`L2531`・`L4863`・`L348`の5件を共通validator合格で回収し、invalid/timeout 0。同一session総時間45.89s→49.35s（+7.5%）、中央値570ms→590ms（+3.5%）
- [x] **48.9 / P1 / #238** 常時`Active`を旧正式RENKIN成功591件へ適用すると10件を失い、route hash 103件が変化。10件を現行`Off`で因果分離すると9件は回復し、常時Active固有の回帰だけでもgate（<=1%）を超えるため既定化を却下。score-only成功を置換しない`retry-on-beam-exhaustion`を実装し、固定58件で同じ5件を回収、invalid/timeout 0、総時間94.05s
- [x] **48.10 / P0 / #237** validなphenyl環のring digitが別の開鎖芳香族原子を覆い隠すL4444欠陥を、既存のatom-level graph-cycle/aromatic-bond検査を全fragment生成経路と完成route境界へ適用して修正。2つの実例fixture、valid aromatic対照、L4444再実行で不正route 0を確認。旧正式成功のうちL4699は不正候補早期除去によるfrontier変化でbeam 100経路を失うが、beam 200で別の有効経路を回収。現行コードの全4,903件正式値は未再測定
- [x] **48.11 / P2** exact ground-truth one-step候補が500-template poolに存在しない12件を500/1,000/5,000/9,974 usable ruleで再診断。local USPTO-50k TRAIN全量から10,553 raw unique / 9,974 usableを再抽出し、既存10,000 tierがlocal語彙上限と一致することを確認。12件中7件は既存ruleのdepth/diversity/coverage代替経路で回収し、5件を未解決として分離
- [x] **48.12 / Issue #239** opt-in `--search-mode recovery`を実装。baseline成功を保持し、具体的integrity拒否→candidate-time element gate、beam到達→diversity、depth到達→depth+1、caller-supplied coverage ladderをfresh frontierで順次実行。全attemptへtrigger・depth/beam・rule count/hash・coverage tier・termination・integrity rejection・elapsedを記録
- [x] **48.13** 比較adapterがrecovery時に`--beam-diversity-slots`を転送しない配線バグを修正。`L87`をdepth 4で再回収し、adapter testを追加。recovery coverage tierをmanifestのstart/end hash対象へ追加
- [x] **48.14** 5,000→9,974 ruleのnarrow-to-broad ladder、tier内diversity retry、coverage+depth+diversity retryを実装。単独因果測定で`L1936`（5k narrow tier）、`L4868`（5k diversity）、`L1214`/`L861`/`L3207`（10k diversity）、`L1301`（5k depth 6 + diversity）を確認
- [x] **48.15** 最新cascadeを固定58件で全件再測定。valid route-to-stock 36/58、route tree/steps/stock/element accounting 36/36、timeout/crash/invalid 0、manifest/arm integrity PASS。wall 997.84s、elapsed p50/p95/max 11.85/54.94/99.95s、peak RSS p50/p95/max 267.1/286.3/331.8MB
- [x] **48.16** 残存22件へdepth 7 + diversityを追加測定し純増0のためblind depth escalationを停止。全22件は最終tierでもbeam/depth到達、16件はunaccounted elementの完成候補のみ、6件は完成候補なし。残存AiZ routeのroot disconnection exact coverageも6/22に留まり、次段階をprovenance-clearedな別corpusまたは別issueでのtemplate abstractionに限定
- [x] **48.17 / Issue #240** license/provenance/leakage境界を先に固定し、AiZynthFinder asset・benchmark route・TEST labelをrule生成/選定に使わず、同一固定TRAINのrdchiral reactant radius=0抽出を研究用higher-level abstractionとして実装。明示H緩和3案は生成0件のためproduction toolingへ残さず却下
- [x] **48.18** 独立固定VAL 200件でTRAIN頻度閾値2/5/10/25を事前登録して評価。`count>=25`はbaseline positive 167件を全て維持し、exact positiveを1件追加、candidate 17,667→25,404（1.438x）、malformed 0でgate通過。決定論的抽出/union scriptとSHA-256 manifestを追加
- [x] **48.19** 選定後に凍結22件holdoutへ最終coverage tierとして適用し、`L1206`・`L2879`・`L2778`・`L3129`の4件を追加回収。route tree/steps/stock/element accounting 4/4、全22 process completed、post-arm verification PASS。累積40/58、残18件
- [x] **48.20 / remaining boundary** 残18件を既存の凍結holdout証跡で再監査。18/18 completed、timeout/crash/invalid output=0、新規valid route=0、完成候補10,433件は全て`unaccounted_target_element`で棄却。実行失敗ではなく候補元素accounting境界が主因と診断した。coverage解決ではないため、追加corpus・別TRAIN-only abstraction・正式4,903件更新・既定化・配布corpus化は引き続き別の事前登録full rerunまで禁止

## Phase 47: Open-issue priority follow-up（実装・検証完了、2026-09-05）

- [x] **47.1 / #72** ring-context guardの既定有効化条件を再確認。必要sidecar/templateがcrate packageから除外され、coverage modeのstage2 sidecarとも非互換なため、既定値変更は配布契約を整備するまで保留し、現行の明示opt-inを維持
- [x] **47.2 / #101** diversity-reserved beamで利用可能な新規familyが予約数未満のとき、未使用slotをscore順で埋め戻す。`Active`が実効beam幅を暗黙に縮小しない回帰テストを追加
- [x] **47.3 / #101** staleだった設計文書をRust/CLI/Python/WASM実装済み・既定値`Off`維持の現状へ同期。既存固定VAL cohort測定はbackfill修正前の履歴証拠であり、将来の既定値変更前に修正版の再測定が必要と明記
- [x] **47.4 / #100** local 5,000-template corpusのサイズ・行数・SHA-256と`renkin doctor templates`結果を再検証。既存provenance noteどおり4,999/5,000 load、loaded templateは全件concrete application可能。元dataset revisionは事後復元不能なためcorpus置換は行わず、将来生成分のrevision pinを維持
- [x] **47.5** offline full workspace test（1,173 pass / 2 ignored）、focused beam-selection test（8 pass）、workspace clippy `-D warnings`、rustfmt、`git diff --check`を通過

## Phase 46: Formal comparison correctness fixes（実装・限定再検証完了、2026-09-05）

- [x] **46.1 / Issue #232** retro生成fragmentのexplicit-Hとsurviving graph valenceの不一致をfail-closedで拒否。formal v1.0.0で発見された`C(C[NH]1CCCC1)OC`を回帰fixture化し、validなaromatic `[nH]`を保持。chematic 1.0.4のvalence validationとRENKINのfragment境界の両方で防御
- [x] **46.2 / Issue #231** zero-step direct-purchase routeを「空の不正tree」ではなく、targetがroot兼stock leafである1-node graphとして正規化。collapsed edge count 0、leaf count 1、deterministic route hashを回帰testで固定し、既存multi-step fail-closed検査を維持
- [x] **46.3** formal cohortの保留2件だけを同一shared-stock設定で再実行。`uspto50k_test#L1394`はzero-step routeとしてparseable/hash生成済み、`uspto50k_test#L1080`は不正amineを含まない3-step routeとしてstructure・stock・directional element-accounting検査を通過
- [x] **46.4** open issue 11件を再棚卸し。core correctnessを最優先とし、既にmitigation済みの#72/#77、外部data依存の#86、optional local corpusだけの#98/#100、長期benchmark/researchの#61/#101はP0修正後のbacklogとして維持
- [x] **46.5** v1.0.1修正版でfrozen 4,903-target shared-stock armを全件再実行。591/4,903（12.05%）対AiZynthFinder 4.4.1の200/4,903（4.08%）、paired差+7.975pp・95% CI [+7.098,+8.852]、route-tree 591/591 parseable、arm integrity verification PASSにより正式公開gateをPASSへ更新

## Phase 45: Literature diagnostics legal-safe boundary（実装完了、2026-09-05）

- [x] **45.1 / Issue #233** CC BY 4.0論文を帰属表示しつつ、外部code/dataを流用しないRust独自実装としてtemplate-ID proxyを監査。3 core route以上で誤っていた正規化とduplicate依存を修正し、duplicate/subset/disjoint回帰testを追加。atom-mapped形成結合CDSとは明確に区別し、検索・rankingは不変
- [x] **45.2 / Issue #234** CC BY-NC-ND 4.0論文から文章・図表・data・metric式・codeを取り込まず、既存Synthesizability Kernel出力だけを構造化する`diagnose_route_feasibility()`を追加。stock endpoint、構造、方向的element accounting、forward validation、evidence/condition coverage、複数limiting step、missing informationを非scalar report化
- [x] **45.3** WO2021229454A1と係属中US20260100252A1の独立claimを予備screening。fragment頻度・descriptor集約・学習SA model・fragment density・reward/penalty・統合SA scoreを非goalとして固定。これはFTO意見ではなく、商用時のjurisdiction/claim別専門家reviewを別途必要とする境界をdocsへ記録

## Phase 44: Search runtime reuse and allocation reduction（実装完了・最終再測定待ち、2026-09-04）

- [x] **44.1** `SearchEngine`を追加し、stock・rules・`PreparedRuleSet`・optional `TemplateBondIndex`を複数targetで再利用。既存free function APIは単発互換wrapperとして維持し、同一targetのroute/stats JSON完全一致を回帰テスト化
- [x] **44.2** `renkin-bench` standard/cascadeと`renkin-mcp`を`SearchEngine`へ移行。MCPは`OnceLock`でprocess lifetime中にassetを一度だけロード。coverage batch向け`CoverageSearchContext`も追加し、Stage 1/2のimmutable compiled stateだけを再利用（frontier/closed/cacheは従来どおりstage/targetごとに独立）
- [x] **44.3** search frontierを`Arc<str>` + interned `Arc<Molecule>`へ変更し、生成済み前駆体の再parse、SA score用parse、frontierのString deep cloneを除去。固定3-target・chematic 1.0.3同条件の局所A/BでCPU user timeを1.02〜1.22倍、wall timeを1.07〜3.30倍改善。RSSは約10〜39%増のため、候補ごと保持ではなくcanonical SMILESごとに一つだけ保持するinternerを採用
- [x] **44.4** 完成routeだけ`ReactionStep`をmaterializeする遅延path表現へ変更。beamで棄却される候補ではrule/template/precursor metadataのdeep cloneを行わない
- [x] **44.5** default `raw_propose`（ring/spectator/element-accounting全off）でruleごとの空診断tupleを作らず直接flatten。局所A/Bでは3件中2件でCPU 2〜6%改善、RSSも約2〜5%低下。全candidate unit test 68件成功
- [x] **44.6** 同一template・同一precursor multisetの重複だけをheuristic/path/heap処理前に除外。代表2,000-template targetでは4,360候補中450件（10.3%）を省略。異なるtemplate間の453件はprovenance保持のため除外せず、hot-loop契約テストでも固定
- [x] **44.7** beam全sortのlinear selection案を実装・局所評価後に撤回。同点時のroute semanticsを保持するfallback判定の追加走査が安定した改善にならず、正式採用しない
- [x] **44.8** 固定disjoint VAL-200 cohortで最終総合再測定を完了（2026-09-06）。RENKIN 1.0.1 + chematic 1.0.7、shared stock、500 templates、depth 5、beam 100、timeout 150sを固定。route-to-stock **21/200（10.5%）**、timeout/crash/invalid/setup error **0**、route/step/tree parseability **21/21**、total elapsed p50/p95 **6.73/39.58s**、peak RSS p50/p95 **21.0/43.9MB**、wall **2,283.11s**。全入力hash不変、200行完全一致、duplicate/malformed/route-hash欠落なしでintegrity **PASS**。専用artifact: `data/comparison/formal_v1.0.1/val_44_8_renkin_shared_stock_{renkin_shared_stock.jsonl,renkin_shared_stock_aggregate.json,renkin_shared_stock_manifest.json}`

## Phase 43: Literature-informed route diversity metric（proxy/exact CDS API実装完了、2026-09-05〜06）

- [x] **43.1** Mrugalla et al. (Journal of Cheminformatics, 2025) の Chemical Diversity Score を、現行Route表現に合わせた `template_disconnection_cds` proxyとして実装。template ID集合のcore-route抽出とJaccard距離を共通Rustモジュール化し、MCP `estimate_diversity` に追加表示。Issue #233 reviewでduplicate排除と3 route以上のall-to-all正規化を是正
- [x] **43.2** atom-mapped Bridge `RouteDocument`向けに`atom_mapped_formed_bond_cds()`を追加。形成結合endpoint集合を決定論的に比較し、未マップ/不正stepは`None`へfail-closed。既存native `Route` JSONとtemplate proxyは変更せず、SynPlanner等のmapping付き入力だけをexact対象とする

## Release Log
- v0.16.0（2026-07-25）: cascade検索・graph sulfonamide/ester cleavage・per-node template ranking・step provenanceタグ付けをリリース。詳細は `CHANGELOG.md` を参照。
- v0.17.0（2026-07-25、PR #51、2026-07-26公開確認）: 下記Phase 33の内容（stable template_id + evidence metadata sidecar）をminor releaseとして公開。PyPI/npm/crates.io/GitHub Release全て`0.17.0`で公開済みを確認済み。詳細は `CHANGELOG.md` を参照。
- v0.18.0（2026-07-26、PR #53）: 下記Phase 34の内容（ReactionExample基質固有evidence records、issue #41 phase 2）をminor releaseとして公開。
- v0.19.0（2026-07-29、PR #59マージ→release/v0.19.0→タグ）: RETROSPECT-inspired candidate reranker基盤（`propose_one_step`独立API、feature schema v1、candidate-pool JSONL exporter+manifest、`scripts/train_reranker.py` LambdaMART学習/評価scriptとbaseline arms A-H、paired bootstrap + machine-judged offline gate）。**実データでの学習・評価は未実施**（self-testのみ）。同時にPR #58（renkin-forward CLI、issue #57）もこのリリースに含まれる。詳細はPhase 35参照。
- v0.20.0（準備中、2026-07-29〜）: PR #55（ORD evidence import/audit pipeline、issue #41 phase 3A）+ PR #62（chematic 0.8.0移行によるapply_retro/run_reactants性能回帰の解消）をminor releaseとしてまとめる予定。詳細はPhase 36参照。

## Phase 42: Security S5/S6 local release gate（v0.62.0相当） ✅ 完了（2026-08-31）

crates.io制限下でも、次回公開前に同じセキュリティ検証を再現できる状態を整備。

- [x] **42.1** `deny.toml` に wildcard dependency拒否とduplicate version警告を明示。現行グラフはlicense／bans／sources検査に成功し、`syn` v2/v3の既知重複だけを警告として保持
- [x] **42.2** `.github/workflows/security-audit.yml` にcargo-deny検査とadversarial regression専用jobを追加。workflow権限をjob単位へ縮小
- [x] **42.3** MCP stdioの不正入力回帰（連続拒否後の回復、parse errorの入力非反映、後続frame保持）を実プロセス境界で固定
- [x] **42.4** `scripts/run_security_regressions.sh` を追加し、cargo-deny・MCP 6件・manifest contract 7件を同一fail-fastゲートで実行可能化。Clippy／fmt／actionlintも確認済み
- [x] **42.5** crates.io、タグ、pushなしでローカルコミット完了（`ff29f5a`、`2d0a036`、`9091feb`、`2075ed3`、`fad33f8`、`eb8bbb8`）。既知の残余riskは`syn` duplicateとadvisory DBを未更新のローカル制約

## Phase 33: stable template_id + evidence metadataサイドカー（issue #41 phase 1）・依存更新・docs/SEO是正 ✅ 完了（2026-07-25）

issue #41 phase 1の実装、backlogだったPR #45（chematic大型依存更新）の評価・マージ、docs全体の正確性是正+SEO対応、v0.17.0リリース準備までを一気通貫で実施。

- [x] **33.1** stable template_id + evidence metadataサイドカー（PR #49、2commit、2026-07-25）
  - `RetroRule`/`ReactionStep`に安定`template_id`を付与（hand-crafted: `rule:<name>`、extracted: `smirks-sha256:<trimmed SMIRKSのSHA-256>`）。ファイル順序・件数に非依存
  - `src/evidence.rs`にevidence metadataサイドカー一式を追加（`ConditionCandidate`/`ReportedYield`/`EvidenceReference`/`ReactionWarning`、schema_version検証、reference_id重複・dangling参照・yield範囲外の拒否、重複template_idをVisitorベースの`deserialize_with`で検出）
  - evidence付与はA*探索本体ではなくpost-processing側で実施（`RetroEntry`をtuple→named structへ変更しつつ、`step_confidence`/`success_probability`と同様の遅延パターンを踏襲、性能劣化なし）
  - CLI `--template-metadata`・`renkin template ids`サブコマンド、Python `templates_path`/`template_metadata_path`を追加
  - **明示的non-goal**: yield予測なし、自動文献検索なし、`step_confidence`/`success_probability`の意味変更なし — evidenceは常にキュレーション由来、RENKINが生成した予測ではない
- [x] **33.2** PR #45（chematic 0.4.30→0.6.0）評価・マージ（2026-07-25）
  - cargo test・nn-scoring build・Pythonホイール+smoke・WASM build・代表分子でのbefore/after JSON diff・固定USPTO-50k小コーパスでのsolved件数/route順/validation結果比較・canonical SMILES安定性を確認
  - 差分は説明可能な改善と判断 → マージ、CHANGELOGへは反映済み（依存バージョンをdocsへハードコードしない方針を維持）
- [x] **33.3** docs/SEO正確性是正（PR #50、5commit、2026-07-25）
  - commit 1「事実+API整合性のみ」→ commit 2「per-pageメタデータ/OGカード」→ commit 3「Guides 4ページ新設（Python/Rust retrosynthesis、reaction evidence、OSS比較）」→ commit 4「README冒頭再構成+CITATION.cff」→ レビュー後追加commit「runtime stock・比較claims是正」
  - 新設: `examples/doc_facts.rs`（rule数・building block数をライブ値として出力）+ `scripts/check_docs_facts.py`（stale figure検出、whole-document disclaimer方式）を`ci.yml`の`docs-check`ジョブとして追加 — 今回発見したclass（古い数値・壊れたAPI例・捏造出力）の再発をPR gate側で防止
  - **ユーザーレビューで発見された3件の主要問題を修正**: (1) building blocks「402固定」claimがwheel実体と不一致（実際は402件ファイル/152件フォールバックの二層構造、wheelには`.smi`同梱なし）→ 両数値をdocsとCIチェック双方に必須化 (2) `validate_forward`が「フル出力を受け付ける」というdocsの記述が誤り（`v["steps"]`必須、トップレベルのみ）→ 実行確認の上修正 (3) ASKCOS比較でv1/v2バージョン混在（v1は開発終了、v2はGitLab移行・MIT）→ リンク・ライセンス修正
  - aspirin.md/druglike.mdの捏造出力（存在しないルールが生成したことになっていたroute）をCLI/Python実行結果の実出力へ差し替え
  - GitHub Topics（11個）をPR外でリポジトリ設定として最後に更新済み
- [x] **33.4** v0.17.0リリース準備（PR #51、2commit、2026-07-25）
  - `[Unreleased]`に後方互換な新機能（33.1）が溜まっていたためminor bump（0.16.1ではなく0.17.0）と判断
  - Cargo.toml/Cargo.lock/pyproject.toml/CITATION.cff/README×3/docs内バージョン参照を一括更新、CHANGELOGの`[Unreleased]`を`[0.17.0]`へ移動
  - 副次修正: release workflowが`npm publish`/`cargo publish`の失敗を`|| echo`/`|| true`で握りつぶしていた問題を修正（既知バージョンのみskip、本物の失敗はjobを落としgithub-releaseをブロック）
  - CI green確認後にmerge → masterのバージョンbump commitへ`v0.17.0`アノテートタグをpush。**この修正で失敗が可視化された結果、`CARGO_REGISTRY_TOKEN`（401→403で失効）と`NPM_TOKEN`（Classic種別のため2FA/EOTPで拒否）の両方が実際に長期間broken状態だったと判明**（crates.io側は直前のv0.16.0リリースでもすでに403だったが旧`|| true`が握りつぶしていた、npm側はv0.1.1以降ほぼ公開歴なし）。ユーザーがトークン再発行（npmはAutomation種別へ変更）後、`gh run rerun --failed`で該当jobのみ再実行し2026-07-26に全publish（PyPI/npm/crates.io）・GitHub Release作成の成功を確認

---

## Phase 34: 基質固有evidence records（issue #41 phase 2）+ v0.18.0リリース準備 ⏳ 進行中（2026-07-26）

- [x] **34.1** `ReactionExample` / `schema_version: 2`（PR #52、5commit、2026-07-26 merge）
  - `src/evidence.rs`に`ReactionExample`型追加：target_smiles/precursor_smiles/conditions/reported_yield/warnings/reference_ids/dataset_record_id/notesを持つ、1件の具体的な反応記録
  - schema_version 1/2の分岐：v1は`examples`使用でhard error、v2は許可。v2ではtemplate-level`reported_yields`もhard error化（基質固有の値がテンプレート単位に漏れることを防止、`examples[].reported_yield`へ強制）
  - `ExampleMatch`/`match_example`：canonical target SMILES一致 + canonical・順序非依存precursor集合一致で`ExactSubstrate`/`TemplateOnly`判定
  - `--format explain`にevidence表示追加：rule-author default conditions（非文献由来と明記）、exampleのexact-substrate優先表示（exact全保持+template-only最大3件）、warnings（template-levelとexample-level両方、それぞれ自身のreferenceを解決）
  - `success_probability`の表現修正（"high step-success probability"等の誤解を招く文言を全箇所"template-frequency route score"等へ修正、JSON field名は不変）
  - **レビュー2ラウンド・5件の指摘を追加commitで修正**: (1) JSON/PythonでExampleMatchが不可視・全exampleが毎stepへclone（ORD大量データで肥大化リスク）→ `ResolvedReactionExample`（`match_kind`+flatten）導入、exact全保持+template-only3件cap、`template_examples_total`追加、参照済みreferenceのみ抽出 (2) condition/yield固有のreferenceが未表示（warning以外は例外扱い）→ 各項目が自身のreference_idsを直下に表示・dedup (3) schema v2でもtemplate-level reported_yieldsが許可されたまま → v2ではhard error化 (4) match_exampleのdoc commentが未実装のstereo-insensitive性を主張 → 文言修正 (5) **2巡目レビューで発覚**: (1)の対応で導入したreference trimmingがv1のreference-onlyエントリ（他から引用されないstandalone citation）を巻き添えで空にし、以前`Some(StepEvidence)`だったものが`None`化する後方互換性regression → `examples`が空のentryはfull referencesを維持するよう分岐、回帰テスト追加
  - advisorレビュー（コミット前、1回目）でも3点発見・同commit内で対応: example-level warningsが未表示、`EvidenceScope::SubstrateSpecific`のdoc commentが陳腐化、docs-check CIジョブ（`doc_facts`/`quickstart`/`check_docs_facts.py`）未確認
- [x] **34.2** v0.18.0リリース準備（PR #53、2026-07-26）
  - v0.17.0はすでに公開済み（PyPI/npm/crates.io/GitHub Release全て0.17.0を確認）と判明 — 当初の「v0.17.0として出す」という前提は誤りだったため再判断し、0.18.0（minor bump、後方互換な追加機能のため0.16.1→0.17.0の判断基準と同一）で確定
  - CHANGELOG `[Unreleased]`（34.1の内容）を`[0.18.0] — 2026-07-26`へ移動、`[0.17.0]`は公開済み実績としてそのまま維持（過去バージョンのchangelog内容は書き換えない）
  - Cargo.toml/Cargo.lock/pyproject.toml/CITATION.cff/README×3/docsバージョン参照を0.18.0へ一括更新
  - release workflow副次改善：`github-release`が`smoke-pypi`にも依存するよう変更（既存は3publish jobのみ依存）、`gh release create`の握りつぶし(`|| echo "Release already exists, skipping"`)を`gh release view`による明示的skip判定へ変更
  - README×3のRoadmapにphase 2 bullet追加（README_zh.mdはphase 1 bullet自体が漏れていたため合わせて追加）
  - CI green確認後、rebase merge → version bump commitへ`v0.18.0`タグ → push、の順で実施予定

---

## Phase 35: renkin-forward CLI是正（issue #57）+ RETROSPECT-inspired candidate reranker基盤 → v0.19.0リリース ✅ 完了（2026-07-29）

- [x] **35.1** PR #58 — forward予測のorder-independent candidate discovery（permutation bug修正）、`main.rs`のstderr警告・strict subcommand CLI・strict route JSON対応。レビュー指摘のCHANGELOG/PR本文修正、docs実出力の差し替えを経て通常merge commit（6コミット履歴保持）でmaster統合。Issue #57をcloseし、forward worktree・remote branchをcleanup
- [x] **35.2** RETROSPECT-inspired candidate reranker基盤（PR #59、Pappala et al. 2026 "RETROSPECT" arXiv:2606.07181から着想、独自実装・上流コード非流用）— commit 1〜5構成:
  - `propose_one_step()`独立API（`find_routes`から切り離した決定的one-step候補提案）、candidate feature schema v1（`extract_features()`、構造/化学整合性/reaction-center-template特徴は常時計算、stock可用性/template頻度は明示的にleakage-safe入力なしでは`missing`）
  - candidate-pool JSONL exporter（`pool_export`、byte-identical再現）+ `PoolManifest`（feature schema version・rules content hash・stock identity記録）
  - group_id/target_id分割、zero-candidate group index、scorer/template provenance、mapped reaction-center特徴修正、pool/feature schema contract強制
  - `scripts/train_reranker.py` — LambdaMART（`LGBMRanker`）訓練/評価script。leakage-safe target-levelハッシュバケット分割、coverage(`targets_with_zero_positive_in_pool`)とranking品質(top1/MRR)を分離報告、baseline arms A-H（データ駆動deterministicスコアラー+train-frozen頻度fit）、paired bootstrap（target_idクラスタ）+ machine-judged offline gate PASS/FAIL、`--self-test`で実データ・lightgbm無しでも全ロジック検証可能
  - **実データでの学習・評価・candidate-pool生成（100/500/full）は未実施** — self-testのみでinfrastructureの正しさを確認した段階（PR A commit 2b/評価・gate決定タスクは`pending`のまま backlog、#33/#35参照）
- [x] **35.3** v0.19.0リリース — PR #59マージ後、`release/v0.19.0`ブランチ作成、Cargo.toml/pyproject.toml等バージョン一括更新、CHANGELOG `[Unreleased]`→`[0.19.0]`、release PR経由でCI green確認しmerge、`v0.19.0`タグ+GitHub Release。crates.io/PyPI等への実publishは前例（v0.17.0/v0.18.0）に倣いdry-run確認までで停止

---

## Phase 36: chematic 0.8.0移行によるapply_retro性能回帰の解消（issue由来ではなく内部investigation、v0.20.0準備中） ⏳ 進行中（2026-07-29）

PR #62。`artifacts/perf_root_cause/`で追跡していたapply_retro/run_reactants性能回帰（chematic 0.4.25→0.4.30、redundant canonical_smiles書き込み+symmetric分子でのcombinatorial cost）の最終解消。

- [x] **36.1** narrow fix（97c87e3 git pin、draft PR #62として先行公開）— redundant-write半分のみ修正、catastrophic outlier（worst target）は12%悪化のまま。draftのまま維持（Ready条件未達のため）
- [x] **36.2** chematic 0.8.0公開確認・移行 — 0.8.0がchematic#193（automorphism-orbit pruning、combinatorial cost側の本丸修正）を含みyankedでないことをレジストリ/tagから直接検証（`cargo info`・sparse index・`git merge-base --is-ancestor`）。git依存→`version = "0.8"`のregistry依存へ戻す。API移行不要（無変更でビルド通過）
- [x] **36.3** correctness differential — canonical SMILES 23fixture差分で0.6.0/0.8.0がdative bond以外すべて一致。**発見**: chematic 0.6.0は`N->[Fe]`と`N<-[Fe]`（構造の異なる分子）を同一canonical文字列`[Fe]-N`に潰す実バグを持っていた（0.8.0で修正済み、chematic#196）。RENKIN自身のdata/src両方にdative bond不使用を確認しimpactゼロと結論
- [x] **36.4** 30-target gate再計測 — **旧baseline(3277.9s)がcontention-inflatedだったと判明**（`lessons.md` L27）。同一セッション・連続でfresh計測: master 0.6.0 1822.4s → chematic 0.8.0 1190.6s（total 34.7%改善、p95 33.8%改善、旧worst target 42.2%改善）。index 17を孤立3回計測しレンジ非重複（noiseでないことを確認）
- [x] **36.5** レビュー指摘2件を追加commitで修正（`dcc8bd8`）— `lessons.md` L28/L29参照。apply_retro counterをperf-instrumentation featureへgate化（release binaryのsymbol tableで確認）、benchmark CLIの引数パースをhard error化、Cargo.lockの`[[package]]`ブロックから厳密にchematic version/source/checksumを取得
- [x] **36.6** PR #62 CI green確認後、通常merge commitでmasterへ統合（`cf809a0`）。post-merge CI（CI/Docs/Security Audit）green確認、branch cleanup済み
- [x] **36.7**（2026-08-12確認、追記のみ）v0.20.0は2026-07-29に公開済み（`CHANGELOG.md` `[0.20.0] — 2026-07-29`）。CHANGELOGのapply_retro counter記述も現在は"available under the optional `perf-instrumentation` feature ... default production path has no counter"と正しくgate化済み（36.5のfix済み内容がそのまま反映されている）——stale記述は残っておらず、チェック漏れだっただけと判明
- [ ] **36.8**（Phase 36後続、backlog）Step 0: 4,907-target正式再計測をpost-PR-#62 master（`cf809a0`）でbaseline再定義（旧d90279aはhistorical化、`lessons.md` L27の教訓を踏まえ preflight hard-gateを事前固定）。まだrun未着手。**2026-08-12時点の注記**: `cf809a0`はv0.20.0以降のリリース（0.21.0/0.21.1/0.22.0/0.23.0、reranker統合含む）6本分古いcommitであり、"post-PR-#62のbaseline"という当初の意図自体がstale——再計測するなら基準commitを現行masterへ更新すべきかを含めて再設計が必要（単純に古いcommit参照のままrunしても意味が薄い）。大規模ベンチのため実行は引き続き未着手・承認待ち

---

## Phase 37: ORD (Open Reaction Database) evidence import pipeline（issue #41 phase 3A） ✅ マージ済み（2026-07-29、PR #55）

- [x] **37.1** `renkin evidence match` CLI — 外部反応レコードをRENKINの安定`template_id`に対して決定的にバッチマッチング（fuzzy/類似度マッチなし、malformed SMILESはレコード単位で`invalid_input`、malformed JSONL行はhard error）
- [x] **37.2** `renkin evidence validate-sidecar` CLI — evidence metadataサイドカーの再検証（invalidなサイドカーを成功扱いさせないためのguard）
- [x] **37.3** `scripts/ord_evidence_audit.py` — ORDコーパス（ローカルダウンロード済み想定、network-free）からschema_version 2サイドカー+auditレポート+再現性manifestを生成。unique template match・単一yield候補・provenance揃いのみ採用、それ以外は理由付きでaudit reportにカウント。同一入力2回実行でbyte-identical出力を確認済み
- [x] **37.4** dataset_id欠損時の会計修正、テストカバレッジ追加（PR #55レビュー対応）
- [x] **37.5** 外部evidence import・ライセンス方針のdocs化（`scripts/README_ord_evidence.md`、reaction-evidence guide）
- [x] **37.6** master最新化してのconflict解消（CHANGELOG/src/lib.rs/mkdocs.yml/README×2でPR #59・forward-quality双方の内容を保持）、post-merge audit（`diff --check`・`diff origin/master...HEAD`でPR #56 MCP内容の非混入を確認）、フル検証マトリクス実施
- [x] **37.7** fresh green CI確認後、通常merge commit（rebase/squash不使用）でmasterへ統合（`12ada87`）。PR #56（MCP dual-era対応）・Issue #41には未着手のまま維持（意図的にスコープ外）

---

## Phase 38: worktree衛生 + forward予測品質ロードマップissue（backlog化） ✅ 完了（2026-07-29）

- [x] **38.1** ローカルworktree完全整理 — 9個のworktree entry（`renkin/.claude/worktrees/`配下の未発見nested worktree含む）を監査し、clean-and-pushedは削除、local-only artifactsは`_renkin-archive/`へchecksum付きで退避、uncommitted変更は`wip/*`branchへ保存。最終的に`renkin/`のみ（後にPR #62作業で`renkin-perf-fix`を再作成→作業後に再度削除、を2周実施）
- [x] **38.2** Issue #61作成 — forward reaction prediction品質のbenchmark/改善ロードマップ（Phase 0〜5、5PR構成案）をtracking issueとして起票。Issue #57は再openせず、実装は一切開始せず（次の実装セッションに委譲、"Refs #61"で参照、"Closes"は使わない）

---

## Phase 39: Issue #101 競争力強化プログラム + reranker実データ学習・runtime統合・配布 ✅ 主要トラック完了（2026-07-26〜2026-08-11、v0.22.0/v0.23.0出荷）

Phase 35で基盤のみ実装しself-testどまりだったreranker（35.2、「実データでの学習・評価は未実施」）を実データで学習・gate通過させ、search本体への非侵襲統合（v0.22.0）、さらにPython公開・batteries-included配布（v0.23.0）まで完走した、複数PRにまたがるプログラム。open-state dominance（39.4、PR #104）は診断の結果negative resultとしてpark確定、adaptive beam等の代替も現時点では着手せず。candidate-generation coverage gap（33.0%がzero-positive）は次cycleへ意図的に分離、v0.23.0には混ぜていない。詳細な体制・既知バグ・意思決定は `ROADMAP.md`（gitignore対象、ローカルのみ、当日更新）を参照——このエントリは要約。

- [x] **39.1** hash-atom `[#N]`テンプレート修正（issue #88/#89/#90/#91、v0.21.1としてリリース済み）— `[#N]`/`[#N:map]`ワイルドカードがapply時に無条件失敗していた問題（500件中217件、43%が影響）を、apply-time-onlyの共有variant展開ヘルパーで修正（PR #89）。直後に発覚した2次バグ（spectator原子への誤ったindependent aromaticity付与）をPR #91で修正。修正後は4アーム全件でinvalid/unparseableルートゼロを確認
- [x] **39.2** beam-width感度gate（2026-08-09完了）— 100-target固定サンプル、Conservative×shared-stock、beam 100/200/300を測定。`route_to_configured_stock` 13→15→15、beam_limit_hitがtimeout以外の未解決行でほぼ100%、L1541がbeam200でのみ解決という非単調挙動を確認 → 「beam幅を上げれば回復する」という単純仮説を否定、candidate-ordering/crowd-out仮説を支持。結論: デフォルトbeam幅は変更しない。次の一手としてIssue #101 Phase C（診断専用計装）へ接続
- [x] **39.3** search crowd-out診断計装（PR #102、マージ済み）— `--search-diagnostics`/`--candidate-trace-limit`。診断のみでsearch/pruning挙動は不変（byte-identical-when-off確認済み）。L1541のbeam感度をcross-path duplicate-state crowd-outと診断
- [x] **39.4** open-state dominance（PR #104、draft、`feat/open-state-dominance-101`）— ✅ **診断完了・park確定（negative result、2026-08-11）**。Round 1で発見された2件のバグ（lifecycle bug: 評価済みだが未探索のghost recordがdominateしうる／measurement gap: 一部targetのみの再計測でp95を語れない）をRound 2で修正済み。Round 2G正式gate: `route_to_configured_stock` 16→17/100、invalid 0、regression 0、timeout 1件、p95 98.2s（5基準中3 PASS・2僅差FAIL）、「promising-but-gate-miss」判定
  - **`L4422` timeout根本原因分析**（`data/l4422_timeout_diagnostics/findings.md`）: Round 2Gの既存gateデータ再分析＋`L4422`単体の無制限リラン2本のみ（新規100-target gateは未実行）。candidate腕は99ターゲット中93で**中央値+45.5%／平均+47.5%遅い**が実際にprune（`open_state_dominated_skipped`）されるのは候補の約14%だけ。`L4422`は無制限で236秒（baseline 96秒の2.45倍）、`rules_attempted_total`+46%・`candidates_generated_before_dedup`+50%（横ばいではない）——単なるbookkeepingオーバーヘッドではなく**探索量そのものの増加**と判明
  - **結論（ユーザー確認済み）**: coverage改善（16→17）とtimeoutリスクは同一メカニズムの表裏——crowd-out解消で多様な状態がbeamを生き残るほど、探索量も計算コストも増える。個別に直せる2問題ではない。**Round 2Gの「promising-but-gate-miss」判定を最終結果として維持、gate閾値は事後変更しない、追加gate再実行なし、PR #104はmergeしない**。3候補案（`Arc<str>`最適化=将来の独立perf PR候補／congestion-triggered delayed dominance=新規探索研究、#104とは別実験／gate再設計=将来のprotocol設計時のみ検討）は全て未着手のまま記録
  - **意義**: 「重複state除去でbeam crowd-outを安く解消できる」という仮説を実測で反証——good negative result。効果はreranker（39.7以降）の方が明確に優れると判断し、資源配分をrerankerの usability/配布側（v0.23.0）へ振り向けた（比較: open-state dominance 16→17・timeout 0→1・latency+45.5% vs reranker 16→20・timeout 1→0・candidate集合不変）
- [x] **39.5** reranker実データ学習・offline gate（PR #105、draft、`feat/reranker-real-data-gate-101`）— Phase 35.2の基盤を実データで学習・評価。Phase 3A（実USPTO-50kラベルでのground-truth監査）〜3E（LambdaMART学習・VAL screening gate PASS・model freeze・formal 4,903-target TEST評価）まで完走。VAL: top1 +11.7pp/MRR +11.3pp/top10 +9.3pp（bootstrap CI確認済み）。formal TEST（frozen modelに対し一度だけ実施）: top1 +12.7pp/MRR +11.9pp/top10 +9.1pp——VALと同程度の改善幅でoverfitting兆候なし。誤り分析: 1,742グループ改善 vs 416悪化（net +1,326）。**このタスクの範囲はoffline candidate-rankingのgateのみ**——runtime統合・route-search benchmarkは意図的に対象外（39.6へ）
- [x] **39.6** reranker runtime統合 ✅ **PASS**（2026-08-10、`feat/runtime-reranker-integration-101`）— 39.5で凍結したLightGBM modelを`find_routes`のhot loopへordering-onlyで配線。candidate集合・cardinalityは不変、`template_bonus`の既存[0.0, 0.2]スケール上のrank由来bonusとして置換（加算ではない）。`--reranker-model`/`--reranker-freq-table` CLIフラグ、OFF時はレガシー完全互換、model/table読み込み失敗時はwarn+fallback（hard errorにしない）、実行中の推論失敗はそのrunだけレガシーへ切替えカウント。C/C++依存なしのfrom-scratch pure-Rust LightGBM text-model reader（`src/reranker.rs`）——Python `lightgbm.Booster.predict()`とbit-exact一致（3000行実データ、max_abs_diff=0.0）確認済み
  - **pre-flightで重要な発見**: `data/beam_sensitivity_gate/`の`route_to_configured_stock=13`（commit `dc4263b`計測）がこのbranch（base commit `8ecac2f`）では再現せず`16`になった——`chematic`/`chematic-rxn` 0.10.0→0.11.0のDependabot依存更新が原因と`CHANGELOG.md`/`Cargo.toml`差分で確認済み（reranker配線由来のnondeterminismではない、binary_sha256照合済み）。以降のpaired比較は`16`を正しいbaselineとして採用
  - **OFF byte-diff検証**: 99/100件が`raw_output_sha256`完全一致。唯一の不一致（L1530）はwall-clock timeout境界のflip（150007.9ms timeout vs 112658.9ms completed）で、コードパスの差異ではないと確認
  - **ON vs OFF paired 100-target gate（正式結果）**: `route_to_configured_stock` **16→20/100（+4、相対+25%）**、regression **ゼロ**（solved→unsolvedへ転落したtargetなし）、新規解決4件（**L1541**——beam感度gateが「beam幅だけでは直せない」と結論した非単調crowd-out caseそのもの、5-step・全leaf in stock・警告ゼロのクリーンな解——に加えL984/L4092/L2603）、invalid/unparseableは両アームともゼロ、timeout 1→0、p50/p95レイテンシ12.4s/85.2s→7.5s/62.4s改善（ただし同一セッション交互実行ではないため参考値扱い、L27）、`nodes_expanded`（両アーム未解決のtargetのみ計測可能）は253→247中央値でほぼ横ばい。`common_structural_warning`率6%→9%上昇は新規解決3件が既存の`unaccounted_target_element`/`stereo_center_count_mismatch`という既知の validator注記を引き継いだだけで、新種の警告や既存クリーンtargetの劣化はなし。決定性は新規ユニットテスト＋実modelでのCLI二重実行で確認済み
  - **GO条件は全て満たされ、PASS判定**
- [x] **39.7** PR #105/#106/#107マージ・v0.22.0リリース・tag/publish（2026-08-10〜11）— PR #105（offline gate、`feat/reranker-real-data-gate-101`）を先にmerge（`401588f`）、runtime統合branchをpost-#105 masterへrebase後、PR #106として提出。**5-agentレビュー（AGENTS.md準拠・浅いbugスキャン・git-blame履歴照合×2・comment遵守）で6件発見、全件修正してからmerge**: (1) `index_rules_by_template_id(rules)?`のhard-error gap（5agent中4agentが独立発見、rerankerの「never a hard error」契約に反する致命的矛盾、`match`+fallbackへ修正）、(2) AGENTS.md違反のMoleculeクローン（`merge_into_candidates`のシグネチャを`Vec<RawCandidate>`→`&[RawCandidate]`へ変更し解消）、(3) 内部不変条件miss時の`.unwrap_or(0.0)`によるサイレントmasking（`.unwrap_or_else(panic!)`へ変更）、(4) `reranker_failures`がCLI JSON出力に一切露出していない（`main.rs`両出力経路に追加）、(5) beam_width>0を実際に踏むテストが皆無（`reranker_under_tight_beam_prunes_safely_and_stays_deterministic`追加）、(6) "ordering-only"文書のoverclaim（beam-pruning相互作用込みの正確な記述へ訂正）。commit `2ff9318`でmerge
  - PR #107（`release: v0.22.0`）: バージョン更新（Cargo.toml/Cargo.lock/CITATION.cff/pyproject.toml/docs）、CHANGELOG（VAL top1 17.22%→28.90%・formal TEST top1 16.40%→29.13%+12.72pp・MRR+11.87pp・top10+9.08pp・bootstrap CI [0.1142,0.1401]・誤り分析1,742改善/416悪化/net+1,326・33.0%(1,618/4,903)zero-positive天井を明記）、全feature flagテスト・wheel/WASM build・`cargo publish --dry-run`・frozen-model runtime smoke（SHA-256照合）を実施しmerge（`a7a5339`）
  - **tag `v0.22.0`をuser承認（「はい、全部publishしてよい」）取得後push**——`release.yml`の完全自動publishカスケードが起動、crates.io/PyPI/npm/GitHub Releaseの4面全てを実際のライブAPIへ直接照会し独立検証済み
- [x] **39.8** v0.23.0プログラム: batteries-included model配布 + Python公開（2026-08-11、PR #108）— v0.22.0出荷時点の既知の限界（CLI-onlyでPython/WASM面なし、frozen modelが未配布）を埋める作業。ユーザーの優先順位付け「① batteries-included reranker → ② Python公開 → ③ candidate-generation coverage gap（別cycleへ）→ ④ PR #104はresearch trackとしてpark」に沿って着手
  - **Python公開**: `find_routes_py`に`reranker_model_path`/`reranker_freq_table_path`追加、CLIの`--reranker-model`/`--reranker-freq-table`と全く同じgraceful fallback契約（pathが片方欠如/読込失敗時はwarn+レガシーfallback、never hard error）。`reranker_failures`をJSON出力へ追加（reranker設定時のみ、未設定時はキー自体が不在）。実wheelビルド＋repo外実行で(a)None/None無害、(b)不正path fallback、(c)実modelでの実際のrerank動作、を確認
  - **batteries-included配布**: `scripts/fetch_reranker_model.py`新設——`model.txt`/`frequency_table.json`をGitHub Release assetからdownloadしSHA-256検証。**自前テスト中に発見・修正した実バグ**: `frequency_table.json`のfreeze_manifest.json記載hashは内部`table`データのhashであり全体ファイルhashではない（`phase3e_export_frequency_table.py`がwrapする前に計算）——naive全体hash比較では常に不一致になるはずだった。file自身が埋め込む`sha256`フィールドを読む方式へ修正し、実データで再検証。ユーザー指示により二重検証化: `frequency_table.json`は全体ファイルhash（新設`release_asset_manifest.json`、download真正性）＋内部tableハッシュ（`freeze_manifest.json`、内容自己整合性）の両方をcheck。model/table自体はUSPTO-50kライセンス不明のためMIT crateへ同梱しない設計を維持
  - **5-agentレビューで7件発見、全件修正してmerge**（commit `5a28272`、PR #108は`e5354ad`でmerge）: 最重要2件は3/5 agentが独立発見——(1) `fetch_reranker_model.py`の`--version`デフォルトがCargo.tomlのcrate versionから決まっていたが、検証対象は`release_asset_manifest.json`独立管理の`release_tag`——次のversion bumpで即座に壊れるはずだった。デフォルトをasset manifest自身の`release_tag`から取るよう修正（crate versionと無関係に）。(2) README/CHANGELOGが「rerankerはv0.22.0出荷」と書いてPython面も含めて主張していたが実際はunreleased、矛盾を修正。他: `fetch_and_verify`の例外処理が`RuntimeError`のみ捕捉で`UnicodeDecodeError`等がcleanup（`os.remove`）を素通りしうる問題を修正、`find_routes_py`が`reranker_failures`を露出しておらずPR #106で一度closeしたobservability gapが再発していた問題を修正、`src/reranker.rs`の「CLI-only, no Python surface」という古いdoc commentを修正
- [x] **39.9** v0.23.0リリース・tag/publish（2026-08-11、PR #109）— version 0.22.0→0.23.0一括更新（Cargo.toml/Cargo.lock（`cargo check`経由、手編集なし）/pyproject.toml/CITATION.cff/docs/api/{python,wasm}.md）。**CITATION.cffの`date-released`不整合を同時修正**（`2026-08-09`のまま放置されていたが実際のv0.22.0リリース日は`2026-08-11`——ユーザー指摘により発見・修正）。CHANGELOG `[Unreleased]`→`[0.23.0] — 2026-08-11`（見出し「usability/distribution unlockであり新たな精度claimではない」、v0.22.0の16→20/100等の数値は既存evidenceとして引用のみ・再計測なし）。README（英語版・日本語版・中国語版全て）のreranker関連記述を実態に合わせて修正
  - フル検証実施: `cargo test --workspace`全green、`--features python`（436/436）・`--features nn-scoring`（450/450）・`--features perf-instrumentation`ビルド、296 Pythonテスト、`cargo tree --duplicates`重複なし、`cargo package --list`（`model.txt`不在／`frequency_table.json`+`release_asset_manifest.json`存在／`fetch_reranker_model.py`不在=設計通り確認）、`cargo publish --dry-run`成功、fresh wheel smoke（repo外・実fetchしたmodelで動作確認）、**crate versionを0.23.0にしてもfetch scriptが依然v0.22.0のasset を正しく解決することを実地確認**（PR #108レビューで発見したバグの release-level確認）、WASM build（0.23.0、無関係のpre-existing warning 1件のみ）
  - PR #109を低影響PR運用（ユーザーが2026-08-11に許可: 「大きな影響がないPRなら私の許可を待たずにマージしてもいい」——release/tag/publishは引き続き別途承認必須）のもとmerge（`a8d56fd`）、post-merge master CI（CI/Docs/Security Audit/CodeQL全部）を独立確認
  - tag `v0.23.0`をユーザー承認（「tag and publish」明示指示）取得後push、`release.yml`自動publishカスケード実行——crates.io/npm/PyPI/GitHub Releaseの4面
- [x] **39.10** README整理（英語・日本語・中国語版、PR #111）— Key Featuresテーブル統廃合（31→20行）とRoadmapセクション再構成（最近出荷/進行中/次/`<details>`過去のマイルストーン折りたたみ、Benchmarkセクションと重複するUSPTO-50k履歴パーセンテージを除去）。squash-merge起因の3-way mergeコンフリクト（`docs/readme-i18n-v0230-reranker`のsquash先9a94099と、そこから分岐した作業branchの親コミットa52c4b9が内容同一でも祖先関係にないため、masterマージ時にREADME_ja/zhで偽陽性コンフリクト発生）を`--ours`で解決・`git diff`で内容不変を確認
- [x] **39.11** Candidate-generation coverage program Phase A: Chemical Space Coverage Diagnosis PoC（2026-08-11、PR #112、merge済み）— 33.0%（1,618/4,903）zero-positive gapをnearest-TRAIN ECFP4 Tanimoto類似度で4binに分解。**結果: near/medium/far間でzero-positive率はほぼ横ばい（30.2〜31.8%）、very-far/OODのみ軽度上昇（42.2%）**——「near-TRAINなのにzero-positive」という当初の仮説（Phase Bを強く支持するシナリオ）は不成立。regenerate pipelineは`manifest_test_formal.json`と完全一致するhashで再現し、1,618/4,903を厳密再現（近似ではない）。全計算は`tools/chemical-space-eval/`という独立nested workspaceに分離（RENKIN本体のCargo.toml/Cargo.lock無変更）、chematic 0.11→0.13のバージョンアップも不要と確認（0.11に`ecfp4`/`tanimoto_matrix`/`top_k_similar`が既に存在）
- [x] **39.12** PR #112レビュー対応（2026-08-11、CONDITIONAL APPROVE→修正→merge）— ユーザーレビューで2点指摘。①「Phase Bのcore premiseを否定しPhase Cを支持」という解釈が強すぎる——whole-molecule距離という一つの根拠は否定されたが、「500→10000で局所reaction-center/disconnection patternの多様性が増える」という別メカニズムは今回の指標では判別不能（見えない）。findings.md/ROADMAP.md/PR本文を「B/Cを判別できない」という弱い表現へ修正。②非commit生データ（`train_reference_products.smi`・`test_target_labels.jsonl`・`nearest_train_tanimoto.jsonl`）とcommit済みreportの間のprovenance chainが弱い（pathとcountのみ）——`nearest_train_tanimoto.jsonl.manifest.json`へ`train_reference_sha256`/`test_labels_sha256`/`output_sha256`/`source_hf_revision`/`renkin_commit`を追加し、`report.py`が生成時に実ファイルとhash照合、`coverage_by_chemical_space_report.json`（commit対象）へprovenance chainを埋め込み。次候補としてPhase A.5（template-count scaling 500/1k/2k/5k/10kをTRAIN/VAL development corpusのcandidate-pool生成のみで直接測定、formal TEST 4,903は使わない）をROADMAP.mdへ記録（未着手・未実装）
- [x] **39.13** Phase A.5: template-count scaling直接診断（2026-08-11〜12、ユーザー明示GO、PR #113、merge済み）— `renkin-pool-gen`のみ（route search・reranker・beam・stock不使用）でVAL全量（4,931 groups）に対し500/1000/2000/5000/9979本（10000本要求、USPTO-50k TRAIN語彙の上限）のnested template setを適用。**結果: zero-positive rate 34.0%→17.6%（-16.4pp）——事前登録済み閾値「10pp以上=Phase B strong GO」を明確にクリア**（ユーザー自身の事前予測「33%→約17%」とほぼ一致）。saturation curveは5000→9979でも+2.23pp改善が継続。500本アームのfull-VAL pool-gen出力hashが既存の`manifest_val_full.json`と完全一致し、regenerate pipeline全体の正確性を独立検証。**運用上の発見**: 5000/10000本アームの単発full-VAL実行が環境側から実行時間約40〜70分の地点で繰り返し強制終了される問題に遭遇——1回のBash呼び出しを250〜500 group単位のchunkに分割し、成功したchunkから逐次結果を追記する方式へ変更することで対応（killされても直前までの進捗は失われない）
- [x] **39.14** PR #113レビュー対応（2026-08-11、2点修正→merge）— ①**論理矛盾の修正**: Recommendationが「10,000超（例: 20,000）も測るべき」と提案していたが、Provenanceでは既に「9,979はUSPTO-50k TRAIN×現行extraction方式の語彙上限」と明記済みで矛盾していた——「curveは語彙上限に到達するまで改善を継続していた」という正確な表現へ修正し、10k超への拡張は別corpus/別template abstraction手法が必要な別実験である旨を明記。②**scientific verdictとproduction verdictの分離**: candidate cardinality（p50 26→84、3.2倍、sub-linear）を根拠に「production readyそう」と読める余地があったが、生成コスト（wall-clock）は同区間で約28〜30倍（500-target単発実測とfull-VAL chunked合計の両方で一致）——「candidate爆発なし」はcorrectness/memoryの観測であってcostの観測ではないと明記し、判定を「Scientific: Phase B strong GO（確定）」と「Production: 9,979をdefault採用する根拠はまだない（未決定）」に分離。次候補として**Phase B.1: coverage/cost frontier optimization**（500→5000で-14.2pp/約14.8倍cost、5000→9979で-2.2pp/約1.9〜2.1倍cost——5000本がsweet spot候補、9979との直接比較が必要）をROADMAP.mdへ記録（未着手）。修正後CI green確認、merge
- [x] **39.15**（2026-08-12、greenlane継続、新規runなし）2点の準備作業。①**コスト起因のattribution追記**（`data/phase_a5_template_scaling/findings.md`）— 既存full_val 5アーム分のmetrics.jsonのみから算出: wall-clock/template比はほぼ一定（0.80〜1.24 s/template、20倍のtemplate数レンジで約1.5倍の変動幅）な一方、wall-clock/merged-candidate-row比は約8.7倍変動——match-timeがdominant costであるという既存findings.mdの推測（「if...via profiling」）を、新規instrumentation・新規runなしで裏付け強化。②**Phase B.1のpre-registration草案**をROADMAP.mdへ追記（承認・開始はまだ、Phase A.5と同じくユーザーの明示GO待ち）——質問設定（pool-gen costではなくroute-search-level cost測定が必要）、候補3本（500/5,000/9,979）、既存beam-sensitivity-gate/reranker-gateと同じpaired-methodology、閾値はプレースホルダー（ユーザー自身が確定させる前提、Phase A.5の事前登録閾値と同じ位置づけ）。両方とも文書追記のみ、コード変更・新規計算runなし

---

## Phase 40: Phase B.1 — coverage/cost frontier optimization ✅ negative resultとして完了（2026-08-12〜13）

Phase A.5「templateを増やせばcoverageが伸びる」の確認を受け、「現行のtemplate matching costモデルで何本まで実用になるか」を検証したプログラム。ユーザー明示GO（10ステップ事前登録）で開始、最終的にnegative resultとして正式closeした。commit `8eb395e`（`.`境界バグ修正、独立commit・回帰テスト付き）、`ae7f769`（Phase B.1 findings + 計装、research commit）。

- [x] **40.1** 発見: retrieval index（`TemplateBondIndex`/`--bond-index`）は新規実装ではなく既存機構（v0.3〜0.7時代から存在、`renkin`本体CLI/`renkin-bench`には配線済みだが`renkin-pool-gen`には未配線）と判明。Step 1 cost attribution計装（`perf-instrumentation` feature gate）を追加し、`raw_propose`（SMARTS match+apply）がコストの98.7〜99.9%を占めると直接計測で確認
- [x] **40.2** retrieval index再設計を2方式試行、いずれも速度gate未達でSTOP。①OR/union方式（既存）: 5,000本で1.14x。②AND/subset方式（新設計）: 1.18x——実サンプルで平均81%のルールが除外されず残ると判明、element-pair粒度が粗すぎることが根本原因。AND方式実装中に`bond_pairs_from_smirks`の`.`境界バグ（複数成分SMIRKSで偽の結合ペアを誤検出）を発見・修正（後に独立commitへ分離）
- [x] **40.3** ユーザー判断で方針転換——indexingでコストを下げる方向を諦め、Phase A.5データから直接production候補を選定。coverage-gain-per-cost frontier分析（既存データのみ、新規run不要）で2,000本を第一候補に選定・ユーザー確認
- [x] **40.4** 2,000本のroute-search gate（500 vs 2,000、reranker OFF）: 150秒timeoutでFAIL（timeout率2%→23%、regression 2%）。原因診断——nodes_expandedはほぼ横ばい（1.04x）、暴走的探索爆発ではなく単純な線形コスト増（per-node cost約4.4x）と特定
- [x] **40.5** timeout枠を600秒に拡大した「Phase B.1b」を新実験条件として実施（150秒版のFAIL判定は不変・確定のまま）。5基準中4つPASS（coverage +3pp、p95 2.36x等）だがregressionは2%で不変——原因はfixed-beam-width-100によるcrowd-out（template増加で候補が増え、既存の解がbeamから押し出される）。ユーザー判断: 原因理解済みでも閾値は免除せず、2,000本を正式REJECT
- [x] **40.6** 次候補1,000本を同条件（600秒/beam100）でpaired gate。500アームとのbinary-hash fail-loud検証で不一致判明（index revert中の再run）→ 両アーム新規再実行。非time系metricsを先に評価する方針（システム高負荷のため）——coverageが+1pp（基準+3pp未達）で即REJECT。regressionは1%（境界だが技術的にPASS）
- [x] **40.7** 1,000本・2,000本ともFAIL → Phase B.1をnegative resultとしてclose。「candidate-pool coverageは増えるが、現行search architectureでは安全にroute coverageへ変換できない」。Step 9 (reranker ON互換性gate) は本番count未確定のため未着手のまま正しく終了。formal TESTは最後まで不使用
- [x] **40.8** コード後片付け: AND-semantics再設計はrevert（既存の`--bond-index`が5,000本の実験でしか検証されておらず、本番の500本route-search用途では独立検証されていないため、未検証の挙動変更を出荷しない判断）。`renkin-pool-gen`への`--bond-index` CLI配線も、今後使う経路がないため削除。保持: `.`境界バグ修正（独立commit、回帰テスト`bond_pairs_from_smirks_resets_at_component_boundary`付き）、cost-attribution計装（`perf-instrumentation` gate、Step 1の発見を生んだもの）、`renkin-pool-gen`のper-target timing計装
- [x] **40.9** 運用トラブル対応: 実行中にディスク枯渇（`ENOSPC`、`/System/Volumes/Data`が100%使用）でBash toolすら実行不可になる事態が発生。ユーザーに状況説明・介入依頼、その後空き容量確認後に本セッション生成の大容量gitignore対象`*_pool.jsonl`ファイル（Phase A.5含む、計約1.9GB）を削除して復旧。`compare_run.py --resume`のper-target flush+fsync設計により、diskクラッシュ含む全ての中断でデータ損失ゼロを確認
- [x] **40.10**（2026-08-14、pre-registration記録のみ、実装・run未着手）ユーザーが次候補として**Phase B.2: Progressive Template Escalation**を提案——Phase B.1の教訓（2,000本の5 gains/2 regressionsは「既存solveを同じ大規模searchへ再投入したことでbeam crowd-outに負けた」ことが原因）から、「500本で解けたtargetは大規模searchへ二度と入れない」段階的fallback設計（Stage 1: 500本で通常探索・solveなら確定 → Stage 2: unsolvedのみ1,000/2,000本で再探索）を着想。3アーム（A: 500のみ／B: 500→1,000 fallback／C: 500→2,000 fallback）、primary gate（coverage>=+3pp、**regression=0固定**（Phase B.1の<=1%より厳格）、invalid=0、決定論的一致）、cost分類（p95<=2.5x=default候補、p95>2.5xでもgate PASS=opt-in coverage mode候補）を事前登録。既存100-target sampleはmechanism replay専用に格下げ、判定には新規disjoint VAL sampleを使う方針。ユーザー指示により**ROADMAP.mdへの記録のみ実施し、実装・run開始前にSTOPして報告**——次の明示的GOを待つ

## Phase 41: Phase B.2 — Progressive Template Escalation ✅ 主要測定完了（determinism replay残、2026-08-14）

ユーザーGO後、3点の追加pre-registration（新規disjoint 200-target sample・hash順選出、Stage 1を1回だけ共有実行、B/C tie-break規則）を反映した上で実測。RENKIN coreへの新規public API追加なし、既存`compare_run.py` CLIのみで実装（benchmark/orchestration layerのみという実装境界を遵守）。

- [x] **41.1** sample設計: 既存100-target・formal TESTを除外し、hash順（SHA-256決定論的）でdisjoint 200-target sample + 別の10-target smoke sampleを選出、run前にSHA-256込みでmanifest固定（`data/phase_b1_frontier/phase_b2_sample_manifest.json`）
- [x] **41.2** smoke test（10件）でStage1→unsolved manifest生成→Stage2 B/C→mergeの機構を検証——Stage2の対象集合がStage1のunsolved集合と完全一致、Stage1 solved集合との重複ゼロを確認してから本番200件へ進行
- [x] **41.3** Stage 1（500本、200 targets、150秒/10秒timeout）を1回だけ実行——33/200 solved（16.5%、Arm A基準値）、timeout 2/200、invalid 0。unsolved 167件をsample-list化しArm B/C共通の入力とした
- [x] **41.4** Stage 2 Arm B（1,000本、167 unsolved、600秒/20秒timeout）——新規5件solve。Arm A比+2.5pp、事前登録閾値+3ppに届かず**FAIL**（regression=0で構造的仮説は成立、coverageのみ未達）
- [x] **41.5** Stage 2 Arm C（2,000本、同じ167件、600秒/20秒timeout）——新規17件solve。Arm A比+8.5pp、regression=0、invalid=0で**primary gate PASS**。ただしend-to-end p95が5.72x（基準<=2.5xを大幅超過）——**opt-in coverage mode候補**（defaultではない）と分類。Stage-2 invocation率83.5%（この200-target sampleでは「稀なfallback」ではなく「大半がescalateする」実態と判明
- [x] **41.6** regression=0を構造・実測の両方で確認（Stage2対象集合がStage1 unsolved集合と完全一致、Arm A solved集合がB/Cのsolved集合の真部分集合）。B/C tie-break規則は両方PASSした場合のみ適用対象——Bが coverage で FAIL したため今回は不要（Cが唯一の合格候補）
- [x] **41.7** 運用: 実行中システム負荷が最大load average 56.83（10コアに対し5.7倍）まで悪化し進捗が大幅に遅延——プロセス自体は生存・計算継続中と確認、無関係プロセスをkillせず待機する方針を継続。`wall_clock_total_sweep_s`がresume回数分fragmentされるため、per-row `total_elapsed_ms`合計から真の累積計算時間を再構成する手法を確立
- [x] **41.8** `data/phase_b1_frontier/findings.md`へPhase B.2セクション追記、`ROADMAP.md`のPhase B.2セクションを結果で更新、memory（`project_renkin_issue101_reranker.md`、`feedback_renkin_pr_discipline.md`）を今回の意思決定パターン（事前登録閾値は理解済みでも免除しない、失敗実験コードは独立価値がなければ残さない）込みで更新
- [x] **41.9**（2026-08-14、ユーザーレビュー後）用語修正: 「fallback」（希少・軽量に聞こえる）表現を"coverage mode"へ統一（findings.md/ROADMAP.mdのArm B/Cラベル、無関係箇所は対象外）。「Stage-2 invocation率83.5%＝ほとんどのtargetが二段目まで走る」という実態を明記済み——rare safety net的な説明を明示的に否定
- [x] **41.10**（2026-08-14）determinism gate設計を確定し、orchestrationを実コード化。`scripts/phase_b2_orchestrator.py`（`unsolved_sample_list`/`merge_arm`（fail-loud invariants）/`semantic_projection`/`projection_sha256`/`verify_consistent_binary`）+ `scripts/tests/test_phase_b2_orchestrator.py`（17 unit tests、既存313件と合わせて全green）。6つの必須invariant（Stage1 solved上書き禁止、Stage2入力集合=Stage1 unsolved集合の完全一致（両方向）、安定したmerge順序、重複target ID即エラー、Stage2結果欠損即エラー、binary/config hash不一致即エラー）をテストでカバー。37-target spot-check構成（Arm C新規solve全17件＋Stage2 unsolved 10件（hash順）＋Stage1 solved未escalation 5件（hash順）＋latency tail/timeout境界5件）を確定・選出済み。semantic projection比較対象フィールド（target_id・stage1 outcome・stage2 invoked・selected stage・route_found・normalized_route_sha256・configured-stock leaves・validator結果・invalid/timeout分類、elapsed time等の非決定的フィールドは除外）を確定
- [x] **41.11**（2026-08-14）reranker compatibility gate（determinism PASS後に着手予定）を事前登録: 新規disjoint 100-target VAL sample（既存200件とは別）、Arm A（500-only+reranker ON）vs Arm C（500→2,000 coverage mode+reranker ON）の2アームのみ。gate: coverage>=+3pp、regressions=0、invalid=0、reranker_failures=0、semantic determinism exact。p95<=2.5x要件は課さない（既にopt-in枠）が実測値は必ず報告、異常な悪化（目安10倍以上）があれば製品化を止める。v0.24までの順序（determinism PASS→reranker compatibility PASS→coverage mode CLI/Python設計→product integration→formal TEST一発確認→v0.24.0）も記録
- [x] **41.12**（2026-08-14、一時停止→再開）ユーザー指示によりCPU作業を一時全面停止（37-target determinism runを0/32で安全に中断、プロセスkill・orphan残存なし確認）→ 再開OK後、`scripts/phase_b2_orchestrator.py`+テスト+decision-run一式をcommit（`1adab83`）
- [x] **41.13**（2026-08-15）determinism gate実行完了——37-target×2回（Stage2 escalated 32件、Stage1-only 5件）、system負荷による複数回のkill/resumeを経て完走。判定: **全37件・両runでsemantic projection SHA-256完全一致→PASS**。Arm C（500→2,000 coverage mode）を正式にproduction candidate（opt-in coverage mode）としてfreeze。結果をfindings.md/ROADMAP.mdへ記録、commit（`e247b7d`）
- [x] **41.14**（2026-08-15）reranker compatibility gate実行——新規disjoint 100-target VAL sample、Arm A（500+reranker ON）vs Arm C（500→2,000 coverage mode+reranker ON、Stage1はArm Aを再利用）。結果: coverage 27→34/100（+7pp）、regressions=0、invalid=0、reranker_failures=0——4基準はstrong PASS。途中で`compare_renkin_adapter.py`が`reranker_failures`を`tool_specific`へ捕捉していないバグを発見・修正（`8ec6eb5`、新規unit test付き）、Arm Aを修正版で再実行。determinismは5-target spot-checkで暫定確認しPASSとして記録、commit（`2a83924`）
- [x] **41.15**（2026-08-15、ユーザーレビュー後）記録訂正: 41.14のdeterminism「PASS」は事前登録した37-targetプロトコルの代替にならないと判断——2,000-templateのregression gateを「原因判明」で免除しなかったのと同じ理由で、5-targetの軽量spot-checkもgateの代替にはしない。findings.md/ROADMAP.mdを「4/5 criteria PASS、determinism PENDING」へ訂正（削除ではなく訂正として追記、履歴は残す）。`phase_b2_orchestrator.py`の`semantic_projection`に`reranker_failures`フィールドを追加（+2 unit tests、計19件green）
- [x] **41.16**（2026-08-15）extended determinism replay完走——同一37-target集合をreranker ON構成（Stage1: 500+reranker 150s/10s、Stage2: 2,000+reranker 600s/20s）で2回実行、`merge_arm`+`projection_sha256`で比較。**結果: FAIL（36/37 exact、1件mismatch）**。事前登録「1件でもmismatchならgate FAIL」を厳格適用し、丸めずFAILとして記録。mismatchは`uspto50k_val#L2330`——run1は600s wallでtimeout（結果未測定）、run2は585.2sでcompleted（route_found=false）。no-reranker版では同targetが360s台で余裕を持ってcomplete（両run一致）していたことから、rerankerのオーバーヘッドがこの1件の実計算時間を600s境界近傍まで押し上げ、システム負荷変動（load average 6〜22で推移）が分類を反転させたと診断。他のtimeout 2件（L2531/L2551）は両runとも600s wallで安定。60秒以内境界に達した非timeout completionはL2330のrun2のみ（次点481.4s/406.8s）——孤立事例であり系統的な600s過小設定ではないと判定。Stage1のsolved/unsolved分割（8/29）は両run完全一致——段階escalationの核心不変条件は健全。findings.md/ROADMAP.mdへ全文記録（丸めない・免除しない）、次の判断（診断的再実行か新条件での再実行か）はユーザー待ち
- [x] **41.17**（2026-08-15、design-only・実装なし）coverage mode設計ドキュメント作成: `docs/design/coverage-mode-v0.md`。既存コード（`src/main.rs`のhand-rolled arg parsing、`find_routes`単一呼び出し、`Output`の`skip_serializing_if`パターン、`fetch_reranker_model.py`のGitHub Release asset方式）を根拠に設計。CLI（`--search-mode standard|coverage`/`--coverage-templates`/`--coverage-timeout-secs`、asset欠落時fail-loud）、Python（`find_routes()`への引数追加、既存関数を分岐しない）、core境界（`find_routes`自体は無変更、呼び出し側で2回呼ぶだけ、Stage1 valid route上書き不可能はcontrol flow構造で保証）、observability（新フィールドは全てcoverage mode時のみ出現、standard mode出力はbyte-identical維持）、timeoutの未解決設計論点（renkinには現在internal timeoutが存在しない——thread+recv_timeout方式を推奨、cooperative cancellationは見送り）、artifact配布（GitHub Release asset+fetch script推奨、Cargo.toml exclude漏れを指摘）、test planを提示。product integrationは41.16のPASSまでcommitしない
- [x] **41.18**（2026-08-15）local masterがorigin/masterより先行していた7 commitを`research/phase-b1-b2-progressive-escalation`ブランチへpush、PR #118をmasterに対してopen、full SHAでremote反映を確認済み。以降の全commit（記録訂正・design doc・extended replay結果・CPU time計測追加・Phase B.2d）も同ブランチへpush、PR本文を都度更新
- [x] **41.19**（2026-08-15、design docへの追記も含む）coverage mode設計ドキュメントを修正: detached thread + recv_timeout案を撤回（Stage2 invocation率83.5%の長寿命呼び出し元でthread蓄積が実害になるため）、cooperative cancellation（`find_routes`へoptional deadline引数、main loopで定期check、timeout時に実際に停止しstats付きで返す）を正式方針に採用。algorithmic semantic determinism（十分なbudgetがあれば決定論的）とoperational timeout classification（wall-clock締切付近はload依存で変動しうる）を製品契約として明示的に分離
- [x] **41.20**（2026-08-15）`scripts/compare_renkin_adapter.py`へCPU user/sys time計測を追加——既存の`/usr/bin/time -l`レポート（peak_rss計測と同じレポート）から追加で正規表現抽出、`tool_specific.renkin.cpu_user_s`/`cpu_sys_s`としてtimeout/no-route/route-found全分岐に反映（crashed/invalid_inputは対象外）。+2 unit tests（318 tests green）。Phase B.2dのCPU time記録要件のため、かつ汎用的に有用なので共有adapter側に追加
- [x] **41.21**（2026-08-15）Phase B.2d「uncensored L2330 semantic determinism diagnostic」実行——`uspto50k_val#L2330`のみ、同一binary SHA/config、3回独立実行、external safety cap 1200s（600s gateの再試験ではなく観測用の診断条件）、開始前に無関係な高負荷processがないことを確認、各runの開始/終了load average・wall time・CPU timeを記録。**結果: PASS**——3/3が1200s以内（実測360.6s/414.4s/477.7s、600s以内でもあった）に完了、reranker_failures=0×3、semantic projection SHA-256完全一致×3。判定: algorithmic semantic determinismは確認、ただし**41.16のFAIL（36/37 exact）は変更せず永続的に記録したまま**——別の問い（十分な時間があれば決定論的か）への回答であり、600s gateの合否を書き換えるものではない。副次的発見: cpu_user_s(~2000-2456s)がwall-clock(~360-478s)の5.6〜6.8倍——内部並列化を直接確認、system load averageがwall-clock timeout境界に効く理由を補強。findings.md/ROADMAP.mdへ全文記録、PR #118を更新
- [x] **41.22**（2026-08-15）PR #118（Phase B.1負の結果＋Phase B.2記録訂正＋coverage mode design doc＋extended determinism replay FAIL＋Phase B.2d diagnostic PASS一式）をCI green確認後にmerge（merge commit `df37005`、ユーザーの明示的事前承認「CI green後にPR #118をresearch/design recordとしてmergeしてよい」に基づく）。全11 CI check pass（Test/Lint/Version sync/Python smoke含む）。origin/masterへfast-forward、branchはGitHub側で削除済み。product integration・coverage mode CLI/Python実装・version bump・asset uploadはまだ未着手——別PRで、ユーザーの新たな明示的GOを得てから開始する
- [x] **41.23**（2026-08-15）**Phase 41.18A: Cooperative Cancellation Foundation実装完了、PR #119をmasterへmerge（merge commit `94782d7`）。Phase 41.18A完了。** `SearchConfig`/`SearchStats`へのfield追加なし（public non-`#[non_exhaustive]` structのため破壊変更回避）、`find_routes`シグネチャ・出力・挙動は完全不変（薄いwrapperへ変更のみ）。新規追加API: `SearchControl`（`unlimited()`/`with_timeout()`/`with_deadline()`、wasm32では時計取得系methodをcfg除外）、`SearchTermination`（`Completed`/`DeadlineExceeded`）、`SearchRunResult`、`find_routes_with_control()`。detached threadは不使用——frontier loop内3箇所（loop先頭・expansion完了直後・child processing各entry前）でcooperative cancellation、`break 'frontier`でloop全体を確実に終了。新規10 unit tests。独立review agent（mutation testing含む）による1回目レビューで4件のfinding（overshoot説明の不正確さ、checkpoint 3の冗長性、checkpoint 1の分離テスト欠如、termination分類の優先順位）を発見——全て同PR内で修正し、同じagentへfocused re-reviewを依頼、4件ともclosed確認（agent自身のmutation testingで独立検証済み）。全CI green（11 checks×2ラウンド）、fmt/clippy workspace/全workspace test/python・nn-scoring feature/perf-instrumentation/wasm-pack build/Pythonスクリプトテスト全て確認済み。post-merge CI（CI/Docs/Security Audit/CodeQL）も全green確認。local masterをorigin/masterへfast-forward、feature branch削除済み。次（Phase 41.18B: shared orchestrator＋CLI/Python surface）は別PR・別GO待ち

---

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
- [x] **31.12b**（Phase 32 / 32.7で完了）validator fidelity分析 — n=300サンプルでInvalid判定695件中575件(83%)相当がatom-balanced済みだった問題を、canonical-SMILES文字列比較の非不変性として特定。VF2 fallbackを立体マーカー時は無効化して導入し、gold set 378 stepでInvalid 293→91、202件を独立再検証、Valid→Invalid逆行0を確認。残るretro-fragment価数問題は別課題

Phase 31のcorrected baseline確立・公開はこれで完了。最終状態・全provenanceは `tasks/phase31_final_remeasurement_run.md` を参照。残る検証精度の課題はPhase 32へ引き継ぐ。

---

## Phase 32: 検証精度の残課題（backlog、2026-07-22 Phase 31から分離）

Phase 31で「壊れた指標を先に直す」は完了。ここからは探索精度そのものではなく、残った検証・計測基盤の課題。

- [ ] **31.8**（Phase 31より移動）cascade Stage2再計測（Stage1未解決分、depth=7 beam=300）— 修正版ルールセット(commit `e20dc8c`以降)に対して未実施。Stage1と結果を分離して報告
- [x] **31.12b**（Phase 32 / 32.7で完了）validator fidelity分析 — canonicalization由来の偽陰性を分離・修正済み。`suzuki_retro`等のrule別残差は、validator fidelity修正後に別途retro-fragment価数・立体・rule仕様の課題として扱う
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
  - **Track D比較完了（2026-07-25）、PR #47（squash `77a330b`）でマージ済み**。実装をcommit（`94db50e`）、origin/masterをmerge（`85f5df0`、Cargo.toml競合は両[[example]]エントリ保持で解消）してPR #37-#40の修正（特にPR #39のvalidator VF2フォールバック）を取り込んだ上で最終比較を実施。PR初回CIで`examples/measure_rank_recall.rs`のcommit漏れ（Cargo.tomlは参照済みだが実体ファイル未staged）が発覚、追加commit（`0fe02c4`）で修正しCI green化。マージ判断: ゴール文書の字義通りの合格基準は未達（0>0）だが全指標で悪化ゼロ・latency実質改善のため、infrastructure保持の価値ありとして採用（ユーザー承認）
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
