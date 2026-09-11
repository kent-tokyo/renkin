# RENKIN Roadmap

更新日: **2026-09-10**

RENKINをAiZynthFinder、Syntheseus、ASKCOSなどの競合を実質的に上回る、実験に使える
retrosynthesis/CASPエンジンへ進化させるための実行計画。この文書でいう
「超える」は、単一のsolved rateを都合よく最大化することではない。比較条件を
固定したうえで、route coverageとroute qualityを競合級まで引き上げ、同時に
速度、再現性、監査可能性、オフライン実行、Rust/Python/WASMへの組み込みやすさで
明確な優位を取ることを意味する。

## Current benchmark position (2026-09-10, v1.0.5 candidate)

The formal VAL-200 shared-stock rerun reports native `route_found` parity with
AiZynthFinder 4.4.1: 134/200 (67.0%) each. The strict common
validator-plus-stock metric is 134/200 for RENKIN versus 123/200 for
AiZynthFinder (+5.5pp point estimate). This is a fixed-cohort result, not a
universal superiority claim. The next performance gate is therefore not
another headline benchmark, but reducing the remaining paired failures while
preserving strict validation and recording every configuration in the manifest.

競合の位置づけは次のとおりである。ChemformerはSMILES seq2seq／事前学習を用いた
template-free寄りの反応・逆合成モデル、RetroGFNは多様で実行可能な候補を生成する
GFlowNet系モデル、AiZynthFinderとSynPlannerは学習モデルと多段階探索を組み合わせた
planner、ASKCOS v2はtemplate relevance/fast filterとMCTSを含むCASPプラットフォーム、
SyntheseusはRetro*・MCTS・PDVNなどの探索と複数の反応モデルを共通API・benchmarkで
比較するフレームワーク、OpenRetroはone-stepモデルの再現可能なbenchmark基盤である。
したがってRENKINは、まず「正しい候補を十分に生成できること」を固め、その後に
ランキング、探索portfolio、実行可能性、条件提案の順で拡張する。

## North-star metrics

すべての比較は、target split、stock、template/data provenance、wall-clock予算、
hardware、並列度、seed、validation policyをmanifestに記録する。native stockの
大小だけで勝敗を決めない。

| 軸 | 主指標 | 合格の考え方 |
|---|---|---|
| Coverage | configured-stock solved rate、top-k route recall | 4,903件の正式TESTで、同一stock・同一予算のAiZynthFinderを上回ること。未達なら原因をcoverage/ranking/searchに分解する |
| Route quality | strict structural/forward validation、step数、SA、stock cost | coverageを増やしてもinvalid/no-op/atom-lossを増やさない。strict passを別指標で報告する |
| One-step | positive-candidate recall@k、invalid product rate、unique candidate rate | plannerの上限であるzero-positive poolを段階的に削る |
| Ranking | top-1、MRR、top-10、paired bootstrap CI | coverageを変えないordering-only gateを先に通す |
| Operations | p50/p95/p99、peak RSS、CPU time、timeout率 | single binaryで再現可能。WASM/Python/CLIの結果契約を一致させる |
| Budget profiles | mode別のcoverage、解決数/秒、time-to-first-route、coverage-time曲線 | `fast`・`balanced`・`deep`を同一manifestで比較し、用途別のPareto frontierを公開する |
| Trust | determinism、audit pass/partial/fail、provenance | 「見つかった」と「化学的に検証できた」を混同しない |
| Lab utility | yield/condition calibration、chemist preference、route execution outcome | 予測値は校正値と不確実性を伴い、根拠のない成功確率を出さない |

## Overall-performance improvement program

G1でconfigured-stock coverageはAiZynthFinderを上回ったが、strict validationとp95
latencyを含む総合ゲートは未達である。以後は次の順序を固定する。

| Phase | 実装 | Gate |
|---|---|---|
| O1 strict correctness | recoveryで得た135 routesをstage別に再監査し、8件のnot-evaluableと25件の構造警告を原因別に修正。検索中のintegrity/forward gateを優先する | strict validator 68%以上、invalid/partial増加なし |
| O2 bounded recovery | baseline成功には追加探索せず、failure diagnosticsがdepth/beamを示す対象だけへ共有recovery budgetを適用 | p95を現行57.95秒から半減、coverage 67.5%以上 |
| O3 search cost reduction | compiled stock/template index、候補数上限、depth/beamの事前判定、重複展開抑制を段階別に測定 | p95 13.1秒以下を目標、determinism維持 |
| O4 paired composite gate | union VAL-200、formal TEST、AiZynthFinderとのpaired bootstrap CIを同一manifestで再測定 | coverage・strict validity・latencyの全軸で勝利 |

### O5 — Search budget profiles

速度とcoverageを単一の固定設定で争わせず、同じplannerに用途別の明示的な探索プロファイルを
提供する。プロファイルは隠れた挙動差ではなく、depth、beam、template budget、timeout、
recovery budgetをversioned configurationとしてmanifestへ記録する。

| Profile | 目的 | 実装方針 | Gate |
|---|---|---|---|
| `fast` | 短時間で最初の実用候補を返す | bounded depth/beam、候補数上限、早期終了、compiled stock/template index | p50/p95とtimeout率をfast budget内で固定し、strict validityを既存baseline以下にしない |
| `balanced` | 通常利用の速度とcoverageの均衡 | `fast`で開始し、未解決対象だけbounded recoveryを一度だけ実行 | fast単独よりcoverageを改善し、deepよりp95/RSSを抑える。CLIの既定候補にする |
| `deep` | coverageとroute品質を最大化 | depth/beam、候補生成、policy ordering、recoveryを拡張 | fixed wall-clock/node budget内のcoverageとstrict validityを最大化し、determinismを維持 |

評価は累計時間だけでなく、同一時間予算ごとの解決数、解決数/秒、time-to-first-route、
coverage-time曲線、p50/p95、peak RSSで行う。`fast`・`balanced`・`deep`を同一target cohortで
測定し、coverageと時間のPareto frontierを作る。深い探索で追加routeを得るための時間を、
短時間モードの単純な平均速度と混同しない。

実装順:

1. `SearchProfile`とmanifest schemaを追加し、既存の明示的なCLI設定へ展開する。既存の
   `depth`・`beam`・`timeout`・`recovery`の意味とlegacy orderingを変更しない。CLIの
   named profile使用時は、schema-versioned `search_profile` metadataに実効値とrecovery
   policyを記録する（実装済み）。
2. `fast`をordering-only・候補集合不変の条件で導入し、同一targetのbaselineとの差分を検証する。
3. `balanced`の未解決対象限定recoveryを接続し、追加探索対象・追加時間・回収routeを監査する。
4. `deep`を候補生成・policy・探索拡張の実験プロファイルとして実装し、strict validationと
   RSSの回帰を確認する。
5. AiZynthFinder、SynPlannerとの比較を各profileで行い、同一wall-clock budgetのcoverageを
   headline、総時間を補助指標として報告する。

比較harnessからnamed profileを各targetへ転送し、per-target rowへ実効metadataを保持する
接続は実装済み。profile間のcoverage・latency・RSS・strict validity測定とmanifest再生成は
まだ独立した実測gateとして扱う。manifestとaggregateにはprofile名・schema version・
configuration IDを保存し、completed rowにmetadataがない場合はfail-closedする。
同一cohortで3 profileを順番に実行し、各armのrows・aggregate・manifest hashを束ねる
`scripts/compare_search_profiles.py`も実装済み。実測値の解釈と公開判定は、生成された
10-target smokeで各profile 1/10 route_found、invalid・timeout・crash 0を確認した。p95 total
elapsedはfast約6.1秒、balanced約15.0秒、deep約120.1秒で、deepのtail latencyが明確に大きい。
このsmokeはprofile配線・manifest生成の確認であり、正式VAL-200のpaired比較や競合優位性の
根拠ではない。
portfolio manifestを検証してから行う。

このphaseの完了条件は、少なくとも二つのprofileで既存baselineを再現でき、profile間の
coverage・latency・RSS・strict validityが同一manifestから再生成できることとする。単一の
profileが全用途で競合を上回ることは、このphaseの前提条件にしない。

### O6 — Audit-native agent bridge and policy-aware stock

**Completed target: O6.5.** O6.1のreceipt基盤を起点に、O6.2〜O6.4の
policy・loss accounting・canonical interchangeを順に積み上げ、最後にagent execution
replay gateまで通過させる。O6.5の完了までは、O6全体を次期リリース候補の完成条件とは
みなさず、各段階の実装・テスト・監査証跡を独立して確認する。

検索アルゴリズムの追従ではなく、MCP/agent実行と企業内stockを監査可能な実行境界へ
引き上げる。最終routeだけを保存せず、どのtool/APIをどのversion・model・引数で呼び、
どの入力と結果からrouteが得られたかを、機密データを漏らさず再現できるようにする。

| Phase | 実装 | Gate |
|---|---|---|
| O6.1 | versioned `AuditReceipt` と親子execution trace。tool/API、version、model、argument hash、task ID、result hash、timestamp、status、failure codeを記録 | receipt ID・content hashが同一入力から決定論的に再生成でき、timestampは実行時情報として分離され、secret・SMILES・stock本文を保存しない |
| O6.2 | ローカル`StockPolicy` engine。internal inventory、vendor allow/deny、price、lead time、region、banlistを合成し、leafごとに`accepted/rejected/unknown`とreason codeを出力 | policy evaluationがfail-closed、入力stock本文を外部送信せず、同一policy hashで再現できる |
| O6.3 | Bridge adapter loss report。field単位で`preserved/normalized/inferred/dropped/unsupported`を記録し、source node ID、reaction provenance、stock provenance、condition情報のround-trip差分を監査 | loss report欠落時はstrict importを拒否し、lossを成功routeやchemical validityと混同しない |
| O6.4 | canonical route importer-exporterを追加。競合formatをgeneric adapterとして扱い、RENKIN audit-routeとpolicy engineへ接続 | round-trip、schema version、loss accounting、strict route validationを通過。競合assetの変換・再配布はしない |
| O6.5 | agent execution replay gate。retro → condition → forwardのtraceを再生し、各receipt hashと最終audit manifestを照合 | deterministic replay、3 phase completeness、途中失敗の分類、機密情報境界、MCP adversarial suiteを通過。O6完了 |

O6は検索routeのsolved rateを直接変更しない。既定のsearch semanticsも変更せず、まず
MCP/Bridgeの監査機能として導入する。受入条件を満たすまでは、agentの最終出力を
化学的妥当性・調達可能性・実験成功確率の証拠として扱わない。

O2の第一歩として、`--search-mode recovery`に`--recovery-timeout-secs`を追加した。
baselineから後続段階まで同じcooperative deadlineを共有し、段階ごとの無制限な時間増幅を
防ぐ。標準modeの挙動は変更しない。

## Competitor comparison lanes

競合比較は一つのleaderboardに混ぜず、各システムの強みを別laneで検証する。

| Lane | 比較対象 | RENKINが証明すべきこと |
|---|---|---|
| One-step model | Chemformer / RetroGFN | 同一splitでtop-k、round-trip、validity、diversity、OODを比較する |
| Planner | AiZynthFinder / SynPlanner / Syntheseus | 同一single-step model、stock、budgetで、solved rate・top-k recall・p95を比較する |
| Platform | ASKCOS | routeだけでなく、fast filter相当の妥当性判定・制約・evidenceの監査可能性を比較する |
| Benchmark | Syntheseus / OpenRetro | dataset、split、前処理、model、searchを交換しても同一manifestで再現できることを示す |
| Integration | Syntheseus | 反応モデル／探索アルゴリズムを差し替えても、同じroute schema・validation・manifestで再現できることを示す |
| Product | 全対象 | CPU-only、offline、WASM/Python/MCP、cold-start、memory、installation timeを別表で比較する |

Syntheseusを単に再実装するのではなく、そこで採用されている「モデルと探索を分離した
比較可能性」をRENKINのroute contractとaudit pipelineへ取り込む。AiZynthFinderや
ASKCOSのnative stock／モデルを使う比較では、データ・ライセンス・前処理をmanifestに
残し、結果をRENKIN固有のstock性能として報告しない。

## Capability tracks to cover

| 目標 | 対応phase | RENKINの実装方針 |
|---|---|---|
| 予測精度・未知反応への一般化 | Phase 2A–2C | Chemformer adapter、RetroGFN-style diverse proposal、temporal/OOD/generalization splitを同一one-step contractで評価 |
| 多段階探索カバレッジ | Phase 3A–3D | AiZynthFinder／SynPlannerのplanner条件を再現し、A*・beam・progressive search・必要時MCTSを同一budgetで比較 |
| CASP総合機能 | Phase 4–6D | ASKCOS相当のvalidation、fast filter、stock/制約、evidence、condition/yield/selectivityを順に統合 |
| 研究用比較基盤 | Phase 0・3A・7 | Syntheseus／OpenRetroの分離可能なmodel/search、固定split、manifest、artifact、auditを取り込む |

## SynPlanner parity and outperformance track

SynPlannerを最も直接的な統合競合と位置付け、機能の有無ではなく、同一データ・同一
stock・同一計算予算でend-to-endの品質を比較する。各subphaseは独立したartifactと
gateを持ち、最後に一つのplannerへ統合する。

| Subphase | SynPlanner相当機能 | RENKIN側の強化点 | Gate |
|---|---|---|---|
| SP1 | 反応データ整理 | schema、standardization、重複排除、atom mapping、split、license、provenanceをmanifest化 | 入力から再生成でき、split leakage 0 |
| SP2 | ルール抽出 | specificity/generalizationを複数設定で抽出し、template ID、適用範囲、atom balance、ring/context安全性を保存 | extraction再現性、invalid/no-op回帰0 |
| SP3 | policy/value評価 | GNNだけに固定せず、LightGBM／ONNX／Chemformer／RetroGFN候補を共通adapterで比較 | top-k、OOD、calibration、latencyをモデル別に公開 |
| SP4 | MCTS統合 | A*／beamをbaselineに、MCTSをfeature flagで追加。policy/value/rolloutを交換可能にする | 同一budgetでcoverage・p95・memoryがbaselineを悪化させない |
| SP5 | ルートクラスタリング | strategic bond、reaction family、stock、route fingerprintで重複を除き、diverse Pareto setを返す | top-k route recall、cluster purity、diversity、chemist preferenceを測定 |
| SP6 | 統合planner | data→rules→models→search→cluster→auditを一つのversioned bundleで再現 | SynPlannerを含む比較対象に対し、coverage・strict validity・再現性・導入コストの複合gateを通過 |

### SP6の複合勝利条件

- **候補:** one-step top-k recall、round-trip validity、novelty、reaction-family diversityを同一splitで比較する。
- **探索:** fixed wall-clock、node/model-call budget、shared stockでroute coverageとtop-k route recallを比較する。
- **品質:** strict structural/forward validation、atom loss、no-op、stereo/ring warning、未評価率を別々に報告する。
- **運用:** p50/p95、peak RSS、cold-start、CPU-only、決定論、途中停止、失敗分類を比較する。
- **CASP:** stock/制約、evidence、condition/yield/selectivity、abstentionをroute品質と混同しない。
- **結論:** 一つのsolved rateではなく、事前登録した複合gateの全項目とconfidence intervalで判定する。

## Competitive thesis and delivery horizons

競合と同じ機能表を埋めることを目的にしない。RENKINの差別化は、Rust-nativeの
決定論的実行、化学的に監査できるroute、低い導入コストを土台に、競合が強い
「候補の広さ」「学習ランキング」「実験支援」を段階的に取り込むことで作る。

| Horizon | 競争上の問い | 到達する製品価値 | リリース判断 |
|---|---|---|---|
| H1: 信頼できるcoverage | 正しい候補が存在しないのか、探索に残らないのか | benchmarkで原因まで説明できるroute coverage | Phase 0/1 gateを通るまで優越性を主張しない |
| H2: 良いrouteを速く返す | 候補を増やしても既存solveを壊さないか | standard/coverage modeとPareto route | regression=0、determinism、timeout分類を満たす |
| H3: 実験に持ち込めるか | stock・制約・化学妥当性を同時に満たすか | source-awareな実行可能route | 欠損値を安全・安価と解釈しない |
| H4: 根拠付きCASP | 条件や収率をどこまで信頼できるか | evidence-linked predictionとabstention | held-out校正とOOD監査を公開できる |

各phaseは、原則として「実装PR → measurement artifact → default/release PR」に
分ける。featureを作っただけではphase完了とせず、gateを通った測定結果を完了条件とする。

## Phase map: exploration to CASP

| Phase | 主題 | 競合に対する勝ち筋 | 出荷単位 |
|---|---|---|---|
| 0 | benchmark contract | AiZynthFinder／Syntheseus／ASKCOSを同条件で比較できる | manifest、failure taxonomy、再現run |
| 1 | candidate coverage | zero-positiveとtemplate適用失敗を分解し、候補の上限を上げる | coverage diagnostics、template bundle |
| 2 | learned ranking | 生成候補を隠さず、top-k品質と校正で上回る | ordering-only reranker |
| 3A–3C | exploration platform | A*／beam／coverageを交換可能・観測可能にする | search contract、trace、Pareto routes |
| 3D | advanced search | 必要性が測定された場合のみMCTS／learned valueを追加 | opt-in search backend |
| 4 | chemical correctness | coverage増加によるfalse positiveを抑える | strict validator、reaction-family gates |
| 5 | executable planning | stock、価格、hazard、制約をrouteへ反映する | source-aware constrained planner |
| 6A–6B | CASP trust layer | evidenceとfeasibilityを条件予測より先に固める | evidence/feasibility contract |
| 6C–6D | CASP recommendations | 条件・収率・選択性を根拠と不確実性付きで提案する | retrieval → prediction → feedback loop |
| 7 | ecosystem | 同じbundleをCLI／Python／WASM／MCPで再現する | release bundle、cross-surface CI |

## Current baseline (v1.0.5 candidate, 2026-09-11)

出荷済みの強み:

- Rust-native A* / AND-OR search、canonical-SMILES based stock identity、graph-rule
  validation、stable template IDsとevidence sidecar。
- 実データで学習・評価したLightGBM candidate rerankerと、Stage 1からStage 2へ進む
  opt-in coverage mode（cooperative cancellation付き）。
- RENKIN、AiZynthFinder、Syntheseus、SynPlannerのroute auditを同じ
  `pass`/`fail`/`partial` pipelineで処理し、reproducibility manifestを残せる。
- CLI、Python、WASM、MCPの複数surfaceと、決定論的なローカル実行。
- Security S5/S6のローカル／CI共通ゲート（cargo-deny、MCP adversarial suite、
  comparison-manifest contract）を整備済み。依存制限中も公開前検証を再現できる。
- VAL-200 shared-stock再測定を完了。RENKINとAiZynthFinder 4.4.1のnative
  `route_found`はともに134/200（67.0%）。共通strict validator＋stockでは
  RENKIN 134/200、AiZynthFinder 123/200（点推定差+5.5pp）だった。ただし
  paired bootstrapの正式優位性判定は未完了であり、普遍的な優越性は主張しない。
- 1.0.5候補では、static TemplatePolicy artifactのschema不一致をfail-closed化し、
  モデル入力を標準化canonical SMILESへ統一。stock membership lookupも検索単位で
  正負ともmemoizeした。これらはroute semanticsを変更しない安全性・ホットパス改善で、
  速度向上率は別途同一条件で測定する。
- `CompiledStockV1`と`PreparedRuleSet`を実装。100,397-entry stockの同一build
  比較でcompiled loadはplain `.smi` loadより約1,005x高速、template applyの
  5,000-call局所gateは19.4x高速。どちらも限定条件のlocal measurementであり、
  full-searchや競合全体の速度主張には拡張しない。
- Route Feasibility Diagnosticsとtemplate-ID proxyのroute-set chemical-idea
  diversityを追加。いずれも決定論的な監査指標であり、実験成功確率とは扱わない。
  なお、mapping付きBridge `RouteDocument`には独立したexact atom-mapped
  formed-bond CDS APIを提供するが、native `Route`のproxyや検索結果は変更しない。

未完了で、今後の上限を決めるもの:

- formal TESTの33.0%（1,618/4,903）がzero-positive candidate pool。template数を
  増やすだけではbeam crowd-outとコスト増が起きるため、coverageは段階的探索で改善する。
- Phase B.1は1,000/2,000-templateの単段拡張を正式REJECT、Phase B.2は200-targetで
  coverage gateをPASSしたがp95が5.72倍で、standardのdefaultではなくopt-inとして凍結。
- coverage modeのCLI/Python product integrationは完了したが、現行correctness
  stateの固定VAL再測定は+1.0ppに留まり、旧+3pp gateを未達。Stage-2 beam拡大も
  latency/RSSとのtrade-offが大きいため、coverageをdefaultにはしない。
- VAL-200の同率native結果とstrict点推定差は、同一セッションのpaired再測定・
  信頼区間・正式publication gateを満たすまで優越性の証明とは扱わない。
- calibrated route confidence、stock-aware planning、条件／収率／副反応モデルは
  未完成。ASKCOSのような機能を先に表面だけ再現しない。

### Union-gap closure program (historical snapshots)

> The measurements in this historical section predate the final 2026-09-08
> native rerun above. Keep them for implementation history; use the formal
> VAL-200 report for current comparison claims.

185,042構造の`eMolecules ∪ RENKIN` union stockによるVAL-200では、共通stock到達が
AiZynthFinder **123/200 (61.5%)**、RENKIN **121/200 (60.5%)**、SynPlanner
**23/200 (11.5%)**だった。直接購入routeを同じconfigured-stock成功として扱った
AiZynthFinder対RENKINのpaired内訳は両者成功95件、AiZynthFinderのみ28件、
RENKINのみ26件、両者失敗51件である。今回のunion条件に
おける結果であり、全データセット・全stockへの普遍的優越性ではない。

今回の差は2 targetsのpaired net gapなので、全体のtemplate数を無制限に増やさず、
AiZynthFinderのみの28件を失敗原因ごとに改善する。初回atlasの追加8件は再診断し、
全件が`depth_zero_stock_route`（target自体がunion stockにある直接購入route）だった。
候補欠落ではなく、空stepのdepth=0 routeを比較adapterが非評価扱いにする契約差である。

| Gap phase | 実装内容 | 成功条件 |
|---|---|---|
| G0 paired failure atlas | AiZynthFinder-only 28件を`no_candidate`、`candidate_crowdout`、`depth_limit`、`stock/normalization`、`validation`へ分類し、再現artifactを保存 | 全28件に主因ラベルがある |
| G1 low-cost recovery | `depth_limit`/`beam`対象だけadaptive depth/beam、prepared template index、compiled stockを適用 | 回収率増、既存87件regression=0、p95/RSS上限内 |
| G2 candidate coverage | `no_candidate`と確認できた対象だけtemplate retrieval、concrete atom variant、fragment leakage修正を適用。現時点で確定no-candidateは0件 | AiZynthFinder-onlyの半数以上を回収、invalid/no-op増加なし |
| G3 ranking and portfolio | `candidate_crowdout`対象にordering-only reranker、diversity reserve、Pareto route selectionを適用 | strict validation回帰なしでpaired rate改善 |
| G4 correctness-preserving gate | G1–G3をVAL-200からformal TESTへ段階拡大し、CI・速度・RSS・determinismを確認 | union VALとformal TESTでAiZynthFinderを上回る |
| G5 native route-found parity | 共通stock到達とは別に、AiZynthFinder-only native route 28件をdepth/beam枯渇、候補crowd-out、generator欠落、順位付けに再分類し、未解決対象限定のbalanced/deep recoveryを適用 | 現行RENKIN 129/200を維持し、同一cohort・同一budgetで135/200以上。strict validation、timeout、determinismに回帰なし |

G1の対象限定測定（AiZynthFinder-only 28件）では、`depth=6, beam=100`は
strict route-to-stock/validator **0/28**、`depth=5, beam=200`は **6/28**を回収した。
深さだけの拡大は採用せず、beam拡大はstandard検索後に失敗対象だけへ適用する
opt-in recoveryとして実装した。CLIの`--search-mode recovery`に
`--recovery-beam-width N`を追加し、baselineがbeam枯渇した場合だけ一度だけ幅を拡大する。
監査結果には`BeamWidth`段階、baseline/recoveryの幅、trigger、terminationを残す。
既存のstandard modeは変更しない。全200件への適用前に、既存成功のregression、p95、RSS、
determinismを再測定する。union stock・schematic 1.0.8の固定28件でのrecovery armは
**6/28 (21.4%)**を回収し、内訳は`ElementAccounting` 4件、`BeamWidth` 2件、
timeout/crash 0件だった。これは過去の`depth=5, beam=200` 6/28を再現するが、
まだVAL-200全体の勝利ゲートではない。

勝利条件はraw route-found率ではなく、`route_to_configured_stock`、strict validation、
paired CI、p95、RSS、timeout、determinismの複合gateで判定する。

G5ではnative `route_found`を補助指標として隠さず報告する。ただし、native成功が共通stock
またはvalidatorを通過するとは限らないため、native route-foundの改善をstrict coverageの
改善と同一視しない。まず既存のAiZynthFinder-only 28件から6件以上を回収することを
実務上の目標とし、既存RENKIN成功のregression 0を必須条件とする。depth=6単独では過去の
対象限定測定で0/28、beam=200単独では6/28だったため、深さ・beam拡大だけでなく、候補の
多様性保持、ordering-only再順位付け、direct generator補完を組み合わせる。
最初のbalanced限定arm（500 templates、bond index、depth=5/beam=100から未解決時だけ
depth=6/beam=200へrecovery、全体timeout 180秒）は、native route_found **6/28 (21.4%)**、
strict stock到達 **6/28 (21.4%)**、wall-clock **168.9秒**、p50/p95 **3.28/15.66秒**で
完了した。これはG5の回収目標に届く有望な候補だが、5000-template正式条件との一致と
既存成功のregression 0は未確認であり、native 135/200の達成とはまだ判定しない。

v1.0.1 candidateとしてローカル実装・個別gate済み:

- 大規模stockを毎回再canonicalizeせず読み込む、versioned・integrity-checkedな
  `CompiledStockV1`形式と`renkin stock compile`。通常`.smi`との意味的一致、破損時の
  fail-closed、100k/1Mでのload-time/RSS gateを通過済み。
- SMIRKSと`[#N]` concrete variantをruleset単位で一度だけcompileし、通常検索、
  candidate-pool生成、ring-contextのmatch列挙／適用で共有する`PreparedRuleSet`。
  target SSSRも全templateで共有し、500 template x 10 targetの旧経路完全一致、
  複数expansionでもparse 1回、release microbenchmark 19.4xのgateを通過済み。
  最終的な出荷判定はworkspace全体のtest/clippy/package/CIを必要とする。

現行の競合比較artifact:

- `data/comparison/formal_v1.0.2/`にunion stock、各ツールのper-target rows、aggregate、
  manifest、SynPlanner検証結果を保存した。union stock自体は185,042構造で、入力hashと
  AiZynthFinder HDF5のround-tripを確認済みである。

## Phase 0 — Benchmark contract and failure taxonomy

**目的:** 勝敗を測れる状態を先に固定する。既存のIssue #66比較とreranker評価を
統合し、再現不能な数字を増やさない。

成果物:

- target split、shared stock、native-stock、time/memory budget、並列度、seedを固定した
  benchmark manifest v1。
- `raw solved`、`strict validated solved`、`route_to_configured_stock`、
  `not_evaluable`を分離する評価レポート。
- 未解決ターゲットを `no_candidate` / `stock_gap` / `search_budget` /
  `validation_failure` / `timeout` / `parser_failure` に分類する診断表。

Exit gate:

- 同じbinary/configを3回実行して、semantic route projectionが一致する。
- 競合比較で、stock差・モデル差・時間差を混ぜた主張をしない。
- 以後の性能PRは、コード変更とmeasurement artifactを同じPRに含める。

## Phase 1 — Candidate coverage ceiling removal（最優先・進行中）

**目的:** plannerに正解候補が渡っていない問題を最優先で削る。ここがAiZynthFinder級
coverageへの最短経路である。

実施項目:

- 既存のbond indexを「作る」ことではなく、coverage modeを含む全candidate-pool生成経路で
  正しく使えるかを検証する。検索コストの98%以上がSMARTS match/applyである現状を前提に、
  index導入だけで勝てると仮定しない。
- extracted templateのparse/apply rejectを原因別に分類し、bare atomic-number、
  H-count、芳香族性、mapping、生成物fragment leakageを修正する。
- high-level/general templateを、原子収支・ring-context・no-op guard付きで段階導入する。
- Stage 1の標準template set、Stage 2のcoverage set、任意のdomain setを同一APIで扱う。
- one-stepのpositive recall@1/10/50/100、unique率、invalid率を保存する。

Exit gate:

- first gateはzero-positive率ではなく、disjoint VALで「coverage増加・regression=0・
  invalid=0」を同時に満たすこと。zero-positive率は診断指標として追跡し、route coverageへ
  変換できない候補を成功と数えない。
- template abstraction、reaction-center/locality、retrieval、progressive escalationを
  別実験に分け、500/1k/2k/5k/9,979の各点でcoverage、cost、crowd-outを記録する。
- strict validation pass rate、p95、peak RSSに悪化上限を設定し、coverage増加だけを
  理由にmergeしない。
- custom template使用時にsilent fallbackを起こさない。

## Phase 2 — Learned one-step proposal and calibrated ranking

**目的:** AiZynthFinder/ASKCOS/Syntheseusの反応モデル・学習ポリシーに対して、RENKINの候補生成と順位付けを
同じ土俵へ置く。学習モデルはcandidate generationを隠さず、provenanceを持つ。

実施項目:

- 現行candidate poolをtarget-level splitで拡張し、template relevance、reaction-center、
  substrate complexity、stock availabilityをleakage-safeに学習する。
- deterministic baseline、LightGBM、必要なら小型ONNXモデルを同じfeature contractで比較する。
- rerankerはcoverageを変えないordering-only modeから始め、search統合は別gateにする。
- top-k recall、MRR、top-1、pairwise bootstrap、OOD（ChEMBL/天然物系）を分けて評価する。
- `success_probability`を実測頻度または校正済み確率として再定義し、未校正値を成功保証の
  ように表示しない。

### 予測モデルの実装順

#### Phase 2A — Model adapter and OpenRetro-compatible benchmark

- one-step proposerをtrait/APIとして分離し、template-based、Chemformer系seq2seq、
  graph/model系、RetroGFN-style diverse generatorを同じ入力・出力・provenance contractで扱う。
- OpenRetroのような固定dataset/split・top-k・validity・重複率・推論時間のbenchmarkを
  取り込み、route searchの成績とone-step精度を別レポートにする。
- model artifact、tokenizer、checkpoint、前処理、ライセンス、hardwareをmanifestへ記録する。

#### Phase 2B — Generalization and uncertainty

- random splitだけでなく、temporal split、reaction-family holdout、substrate scaffold holdout、
  USPTO-MIT/ChEMBL等のOOD splitで評価する。
- `top-k recall`だけでなく、round-trip validity、atom/mapping integrity、novelty、diversity、
  calibration、abstention rateを必須指標にする。
- 未知反応に対しては、無理にprecursorを生成するより `unknown`／`abstain` を返す挙動を評価する。

#### Phase 2C — Diverse proposal and search integration

- RetroGFN-styleの多様候補を、単純な候補数増加ではなく、reaction-family diversity、
  feasible-round-trip率、route coverageへの寄与で評価する。
- Chemformer等のtemplate-free候補は、forward replay・validation・stock checkを通ったものだけ
  plannerへ渡す。生成モデルの尤度をchemical success probabilityと解釈しない。
- search integrationはordering-only、candidate substitution、hybridの3段階で比較し、
  baseline routeのregression=0を必須条件にする。

Exit gate:

- formal TESTでtop-1/MRR改善のCI下限が正、top-10とinvalid率に事前定義した回帰上限を超えない。
- inferenceが決定論的で、モデルなし時のlegacy orderingを壊さない。
- validation/coverageの改善をrankingの改善と混同しない。

## Phase 3 — Search portfolio and Pareto route selection（基盤実装済み・拡張待ち）

**目的:** 一つの探索設定に全てを賭けず、A*、beam、MCTS相当の探索 budgetを、同じ
one-step proposerと共通のroute contractの上で比較できるようにする。

実施項目:

- deterministic A*を基準に、既に実装したprogressive template escalationをcoverage mode
  として安定化する。SyntheseusにあるRetro*／MCTS／PDVNとの比較可能性を意識して、
  adaptive beam、diversity-aware frontier、Rust-native MCTSは、固定budgetのpaired benchmarkで
  必要性が証明された場合だけ追加する。
- routeをcoverage、steps、SA、stock cost、hazard、confidence、diversityのPareto setとして返す。
- cooperative cancellationとtimeout classificationをCLI/Python/MCP/WASMの契約へ広げる。
- duplicate route、shared intermediate、symmetry、precursor orderを正規化する。

### 探索の実装順

### Phase 3M — Model adapter contract and provenance（今回着手）

**目的:** AiZynthFinder型、SynPlanner型、Chemformer型、RENKIN内製モデルを同じ監査境界
で比較できるようにし、モデルの出力をroute成功と混同しない。

実装順:

1. `ModelManifest`でmodel kind、artifact SHA-256、input feature schema、output semantics、
   template-set hash、ID mapping、training split、license、推論resource上限、OOD abstainを
   固定する。今回このschemaとvalidationを追加し、既存探索経路は変更しない。
   bounded local JSON loaderも追加し、サイズ制限・JSON schema・hash・template-set条件を
   一括検証できるようにした。
2. `TemplatePolicy`をordering-onlyで接続する。候補集合・構造検証・stock判定・forward
   validationはRENKIN側に残し、modelなし時のorderingを完全に維持する。`TemplateInfo`、
   `TemplatePolicyDecision`、`TemplatePolicyPrior`を追加し、abstain・未知ID・不正scoreは
   `FrequencyPrior`へfallbackするadapterまで実装済み。
   固定score-tableをhash検証して読み込む`StaticTemplatePolicy`も追加し、実モデルなしで
   同一targetのpolicy有無をpaired A/Bできる参照adapterを用意した。
   既存`nn-scoring`のONNX logitsをstable template IDへ変換する`OnnxTemplatePolicy`も
   追加した。従来の`--scorer`によるtop-K候補削除経路は変更せず、adapter経由では候補を
   削らないordering-only動作に限定している。`--scorer-ordering-only --scorer <path>`で
   このadapterをCLIから明示的に選べるようにし、通常の`--scorer`は従来どおり維持した。
   CLIと比較adapterにもmanifest/artifact指定を接続し、片方だけの指定やhash不一致は
   実行前に拒否する。既定のlegacy orderingは変更しない。
   CLI integration testで、hash検証済みpolicyが実際のordering経路へ接続され、route JSONの
   契約を壊さないことを固定した。次は同一VALをlegacy/policyの2 armで走らせ、strict・
   p95・RSS・determinismを比較する。
   さらに`ReactionPrior::prior_for_template`を追加し、既存のname-based実装を壊さず、
   model policy側ではstable template IDを優先して参照するようにした。同名ruleやID変換
   がある場合でも順位付けの対象を取り違えない。
   同一cohortの2 armを`compare_policy_ab.py`でpaired集計できるようにした。target IDの
   重複・SMILES・comparison mode不一致は拒否し、route_foundとstrict validator確認率を
   分離、共通測定だけのlatency、arm間route content一致率、McNemarとpaired bootstrap
   CIを出力する。arm間一致率を同一armのrepeat-run determinismと混同しない。RSSもlatencyと
   同じく両armの共通測定だけで集計する。これは評価器の
   実装完了であり、実データの性能改善を意味しない。repeat-run JSONLを追加指定した場合は
   同一armの結果一致率とroute hash一致率も独立集計する。
3. 同一target・stock・template setでpaired A/Bを実施し、strict coverage、候補消失率、
   p50/p95、RSS、determinismを記録する。改善が確認できるまで候補削除やMCTSへ進まない。
4. `ValueModel`をheuristicの交換点として追加し、calibration・OOD・abstentionを別gateにする。
   既存の`MoleculeValueEstimator`にchecked評価を追加し、非有限・負値を安全なSA
   heuristicへfail-closed fallbackする境界まで実装済み。モデル本体、calibration、OOD
   gateは未実施で、paired A/Bの後に進める。構造化`ValueModel`と既存A*スロットの
   `ValueModelEstimatorAdapter`も追加し、confidence/abstainを保持したまま安全に接続できる。
   hash-pinned JSON fixtureを読む`StaticValueModel`も追加し、output semanticsが
   `cost`であること、値・confidence・abstainの境界をロード時に検証する。
   CLIの`--value-model-manifest`/`--value-model-artifact`から既存A*の
   `ValueModelEstimatorAdapter`へ接続し、未指定時のlegacy heuristicは変更しない。
   比較harnessにも`--scorer`を追加し、`nn-scoring`付きbinaryでONNX
   `ordering-only` armを同一target/stock条件に載せられる。3-target smokeで
   model load・route schema・strict audit・resource計測が通過した。
   さらに`--scorer-ordering-blend 0..1`を追加し、モデル順位と既存frequency priorを
   候補集合を変えずに混合できるようにした。既定値は1.0で既存のONNX ordering-only
   と同じ。3-target smokeではbaseline 1/3、ONNX ordering-only 1/3で、coverage改善は
   まだ確認していない。次のpaired測定までは性能向上を主張しない。
   blend=0.5の追加3-target smokeも1/3で、timeout/crashは0件だった。小標本では
   coverage改善を確認できず、blend値の採用判断は正式paired測定へ保留する。
   探索側には`BeamDiversityPolicy::Adaptive`を追加した。純粋なscore beamが少数familyに
   集中し、下位に未代表familyがある場合だけ予約枠を有効化するオプトイン実装であり、
   `off`を含む既存policyの既定挙動は変更しない。効果判定はpaired測定待ちである。
   同一3件のadaptive smokeはcoverage 1/3、strict route 1/3、timeout/crash 0件で、
   既存smokeからのcoverage改善はなかった。候補生成を変えない探索側の小改良だけでは
   このサンプルの未解決対象を回収できないため、次は直接生成候補とcandidate-poolの
   coverage gapへ進む。
   `SearchConfig::retro_generator`とchecked proposal-to-frontier変換を追加し、直接生成
   候補をnative expansionへaugmentできるようにした。未指定時は従来経路を維持し、
   generator出力はdedup・heuristic・stock・route integrity境界を通過する。実モデルの
   adapter接続とcoverage効果は未測定で、次の段階で行う。
   CLIからhash-pinned `StaticRetroGenerator`を読み込むmanifest/artifact指定も接続した。
   これにより、実モデルを同じ境界へ載せる前に、再現可能な直接生成fixtureで探索統合を
   検証できる。実モデルの変換・coverage測定は未実施。
   CLI E2E fixtureでnative候補とdirect-generator候補の共存、checked validation、
   `direct_generator` routeの返却まで確認した。実モデルの候補変換と欠落対象での
   coverage改善は未測定。
   StaticRetroGeneratorのtarget索引はstockと同じstandardize後canonical identityを使い、
   表記違いによる直接生成候補の取りこぼしを防ぐ。呼び出し境界では元のtarget表記へ
   戻してtransport contractを維持し、同一canonical keyの重複fixtureはロード時に拒否する。
   `scripts/prepare_static_retro_generator.py`で、外部モデルのtarget/precursor JSONLを
   hash-pinned artifact/manifestへオフライン変換できるようにした。confidence範囲、
   candidate ID重複、空SMILESは変換時に拒否し、化学的妥当性は従来どおりRust側へ委譲する。
   速度面では、同一target・同一stockのdebug CLI smoke（5回）で通常SMILES読込の
   wall time 0.34--0.46秒に対し、同じ内容の`.rstock`は毎回0.02秒だった。これは
   起動時のstock parse削減の証拠であり、AiZynthFinderとの総合速度比較や探索本体の
   優位性を示すものではない。競合比較ではcompiled stockを使用したarmを明示する。
   比較harnessのconfigured-stock読込も`.rstock`のmagic/headerを除いたpayloadだけを
   取り込むよう修正し、速度armのstock監査件数がartifact headerに汚染されないようにした。
   release binary・compiled stock・depth=3・beam=100の3-target smokeでは、bond indexの
   wall timeはbaselineより一貫して短くならず、target依存の揺れがあったため既定値には
   昇格しない。速度優先の次の本命は、targetごとのプロセス起動を避けるbatch/persistent
   asset再利用であり、AiZynthFinderと同じ実行単位を揃えた後に評価する。
   `renkin-bench --limit N`を追加し、既存入力を一時ファイルなしで先頭N件に制限できる
   ようにした。SearchEngine共有の10-target・depth=2 smokeでは、通常`.smi`のwall
   time 0.80秒に対し、同一内容の`.rstock`は0.23秒（約71%短縮）だった。これはbatch
   起動／stock読込の改善で、探索本体のtargetごとの速度差や競合優位性ではない。
   探索loopでは既にcanonicalなfrontier SMILESをcache hitのたびにowned Stringへ複製
   していたため、borrowへ変更した。cache保存とroute記録でのみ必要なcloneを行い、
   search semanticsは変更しない。search関連106テストとdirect-generator E2Eが通過した。
   さらにrequired-elements filterのtarget maskをSMILES再走査から保持済みMoleculeの
   atomic number走査へ変更した。候補集合の判定は同じまま、展開ごとの文字列走査と
   parse依存を減らした。candidate 68件、search関連テストが通過している。
   bond-indexでactive ruleが32未満になった場合はRayon固定費を避けて逐次処理する
   fast pathを追加した。32以上は従来どおり並列、結果順と候補集合は不変である。
   `--speed-profile`をCLI・`renkin-bench`・比較harnessへ追加し、速度armだけで
   bond indexを明示的に有効化できるようにした。既定armは不変で、coverage/recovery
   との併用は拒否する。speed armの効果はtarget依存のため、採用判断はpaired測定待ち。
   release・compiled stock・10-target・depth=2の3反復ではbaselineとspeed armがともに
   約0.30秒で、今回の小標本では追加のbatch短縮は確認できなかった。速度改善の主効果は
   引き続き`.rstock`のロード短縮であり、template適用の最適化はより大きいsampleで再評価する。
   timing diagnosticsの10-target再測定では、探索合計147.4msのうちtemplate expansionが
   68.6ms（約46.6%）、retro-cache missは73件だった。speed armはexpansion 79.5ms、
   平均探索16.9msで、今回のsampleでは改善せず、bond indexの既定化は見送る。
   release profileにThinLTOとcodegen-units=1を設定し、workspace crate間を含むLLVM最適化を
   強化した。debug/test profileと探索 semanticsは変更しない。release benchmark binaryの
   1-target実行を確認済みで、実効速度差は同一条件の再測定で判定する。
5. `RetroGenerator`は直接生成候補のmapping/provenance/重複契約を定義してから追加する。
   `RetroGenerationContext`、`RetroProposal`、`GeneratorProvenance`、abstainとtransport
   validationを追加済み。Chemformer等のモデル実装・atom mapping・探索接続は未実施で、
   既存のtemplate proposerは変更していない。checked呼び出しで検証を通過しない出力を
   探索境界へ渡さない契約も追加済み。checked経路ではSMILES/mappingのparse検証も必須。
   既存one-step proposerを包む
   `RuleBasedRetroGenerator`を基準adapterとして追加し、candidate ID/source provenanceを
   新契約へ変換できる。`AtomMapEntry`によるmapping証跡も定義し、重複map番号と
   precursor index不正を拒否する。hash-pinned JSON fixtureを読む
   `validate_chemistry`でSMILESとmapping atom indexも検査するが、反応成立・stock・forward
   replayは判定しない。`proposal_to_route`で既存監査入力へ変換できるが、これは形式変換
   であり、successやexperimental probabilityを付与しない。`audit_proposal`で
   その変換結果を既存のnormalizer/audit境界へ通せるようにし、直接生成候補も
   native routeと同じ構造・stock・forwardの監査結果だけを参照する。
   `audit_proposal_checked`ではtransport/SMILES/mapping検証を先に強制する。
   `StaticRetroGenerator`も追加したが、Chemformer等の実モデル接続と検索統合は未実施。
6. 最後にpolicy/valueを使うMCTSをfeature flagで導入する。

Exit gate:

- manifest validationが失敗したモデルを比較へ投入しない。
- policyの呼び出しが標準化canonical targetごとにcacheされ、SMILES表記差でも重複推論しない。
- modelなし時のlegacy orderingとroute検証結果に回帰がない。
- model効果、探索効果、検証効果を同一artifact内で分離して報告できる。
- 外部モデルのlicense・再配布・学習データ・特許リスクが未確認の間は実装・同梱しない。

#### Phase 3A — Search contract and observability

- `SearchProblem`、one-step proposer、inventory、heuristic、termination、route scorerの
  境界を分離し、Syntheseusのようにモデルと探索を交換可能にする。
- node expansion、model call、candidate rejection、stock hit、validation、timeoutを
  同じtrace schemaへ記録する。traceを保存してもrouteの勝敗集計へ混ぜない。
- cancellation、iteration/time/node budget、seed、parallelismをCLI/Python/MCP/WASMで
  同じ意味にする。WASMのworker停止とRust内部のcooperative cancellationを別に検証する。

#### Phase 3B — Portfolio baseline

- A*、beam、progressive coverageの同一proposer比較を先に完成させる。
- beam幅、depth、template budget、time budgetのresponse surfaceを測り、固定値1個を
  “最適設定”と呼ばない。
- standard modeとcoverage modeのroute集合を、coverage gain、regression、cost、p95、
  Pareto hypervolumeで評価する。

#### Phase 3C — Diversity and shared-intermediate search

- 同じ中間体を共有するAND/OR状態を正規化し、duplicate展開とsymmetryによる探索浪費を削る。
- top-1だけでなく、実質的に異なるrouteのtop-k recall、template-family diversity、
  shared-intermediate率を測る。
- diversity rewardはcoverageを隠す目的では使わず、同一coverage下のroute選択に限定する。

#### Phase 3D — MCTS / learned value（必要性が証明された場合）

- SyntheseusのRetro*／MCTS／PDVN、AiZynthFinderのMCTSとの比較を、同一single-step
  model・stock・budgetで実施する。
- MCTSを追加する場合も、探索本体を先に増やさず、A/B可能なfeature flagと共通traceで導入する。
- value modelはroute成功確率と混同せず、calibration、OOD、abstentionを別途gateする。

Exit gate:

- 固定budgetでsingle bestだけでなくtop-k route recallとPareto hypervolumeを報告できる。
- 速度改善がsemantic determinismやstrict validationを壊さない。
- 1 targetの勝ち筋を全体の勝利と誤認しないpaired testを採用する。

## Phase 4 — Chemical correctness and reaction realism（Phase 1と並行）

**目的:** 候補数ではなく、化学者が検討できるrouteへ近づける。false positiveを削る
ことをcoverageと同等のKPIにする。

実施項目:

- 未修正のatom-loss/overmatchを先に閉じ、graph-based rule追加はその後に一反応族ずつ行う。
  coverageを増やすための無制約なrule追加はしない。
- atom mapping、element accounting、forward replay、stereo、ring topologyを一つの
  reaction validation contractへ統合する。
- reaction familyごとのnegative testsと、テンプレート単位の境界ケース監査を自動化する。
- reaction conditionsがない段階では、条件を捏造せず `not_evaluable` を返す。

Exit gate:

- zero atom-loss/no-op routeを既知fixtureとhalide/carboxylation境界ケースで保証する。
- strict validationを通らないrouteが上位に来る場合、その理由がmachine-readableである。
- 新規ruleごとにcoverage delta、invalid delta、p95 deltaを記録する。

## Phase 5 — Stock-aware, cost-aware, and constraint-aware planning

**目的:** 「合成できる」から「この研究室／調達条件で実行できる」へ評価を進める。

実施項目:

- stockをcanonical identityだけでなくsource、価格帯、lead time、hazard、最小包装量、
  availability timestamp付きのsidecarとして扱う。
- budget、禁止官能基、最大step、保護基、スケール、危険試薬、house stockを制約として受ける。
- stockの鮮度とライセンスをmanifestに記録し、価格を永続的な事実として埋め込まない。
- 同一targetで「最短」「最安」「安全」「多様性」のPareto routeを返す。

Exit gate:

- 制約違反がrouteに混入せず、制約なしlegacy modeはbyte-compatibleである。
- shared-stock比較で、stockサイズを揃えたroute coverageとcostを同時に報告できる。
- price/hazard欠損時に、欠損をゼロや安全と解釈しない。

## Phase 6 — Conditions, yield, selectivity, and uncertainty（CASP実験支援）

**目的:** ASKCOS級の実験支援機能を、根拠・校正・不確実性付きで追加する。これは
coverageとvalidationが安定してから着手する。

実施項目:

- ORDおよびライセンスを確認した反応データから、template-levelではなくsubstrate-awareの
  condition/evidence recordsを拡張する。
- reagent/solvent/catalyst、temperature/time、workup、reported yieldを候補集合として返す。
- yield prediction、reaction success、selectivity、side-reaction riskを別モデル・別ラベルにする。
- calibration curve、abstention、OOD検出、evidence referenceをUI/JSON/CLIで可視化する。

### CASPの実装順

#### Phase 6A — Evidence and reaction-step contract

- template、基質、試薬、溶媒、触媒、温度、時間、workup、収率、選択性を別フィールドで
  表現し、template-levelの根拠をsubstrate-levelの実績と混同しない。
- 文献実績、データセット由来のラベル、モデル予測、chemist overrideを出所付きで分ける。
- route全体のconfidenceは、step confidenceの単純な積にせず、欠測・not_evaluable・OODを
  失敗とは別に表現する。

#### Phase 6B — Feasibility and fast-filter gate

- ASKCOS fast filter相当のstep feasibility判定を、forward replay、atom accounting、
  reaction-family negative testと組み合わせる。
- binary pass/failだけでなく、invalid、not_evaluable、低信頼、要レビューを返す。
- route rankingへ統合する前に、one-step precision、recall、校正、false-positiveの
  反応族別集計を固定する。

#### Phase 6C — Condition and yield recommendation

- まず evidence retrieval（類似基質・反応族・一次文献の候補提示）を出荷し、その後に
  条件ランキング、yield predictionを追加する。
- reagent/solvent/catalystとtemperature/time/workupを一つの自由文ではなく、検証可能な
  候補集合として返す。根拠がなければ `abstain` とする。
- reported yieldとpredicted yieldをUI、JSON、CLIで明示的に分離する。

#### Phase 6D — Selectivity, side reactions, and closed-loop learning

- chemoselectivity、regioselectivity、stereoselectivity、side-reaction riskを別ラベルで
  学習・評価し、yieldに埋め込まない。
- 実験結果を任意のchemist feedbackとして取り込めるversioned recordにし、モデル再学習時の
  data leakageを防ぐ。
- held-out substrate、temporal split、OOD splitで calibration、AUROC/AUPRC、coverage、
  abstention rateを報告する。予測が改善してもroute validityが悪化すれば出荷しない。

Exit gate:

- held-out substrate splitで校正指標とcoverageを公開する。
- 根拠のない条件・収率を出さず、予測と文献実績を明示的に区別する。
- route rankingが条件モデルのリークで不当に改善していない。

## Phase 7 — Reproducible product and ecosystem lead（継続）

**目的:** 研究prototypeではなく、競合が置き換えにくい配布・監査・統合基盤にする。

実施項目:

- versioned model/template/stock/evidence bundleとcontent hashを一つのrun manifestにまとめる。
- CLI、Python、WASM、MCPでschema・timeout・validation verdictを一致させる。
- CPU-only single binary、wheel、WASMのrelease smokeとcross-surface differential testをCI化する。
- AiZynthFinder/ASKCOS/Syntheseus/SynPlannerのroute importerと、監査結果の比較可能な公開形式を整える。
- 4,903-target正式比較、OOD、repeatability、memory、cold-start、installation timeを定期公開する。

Exit gate:

- 競合との比較表では、実測値・条件・日付・provenanceが揃わないセルを「未測定」とする。
- 同じ入力とbundleから、CLI/Python/WASMのsemantic output hashが一致する。
- 主要リリースごとに、性能向上と化学的正しさの両方の回帰報告を出す。

## Security track — S0〜S6（全phaseに横断）

「セキュリティの穴を無くす」は、既知CVEを消すことだけではない。RENKINが受け取る
SMILES、SMIRKS、route JSON、template/stock/evidenceファイル、MCPメッセージをすべて
敵対的入力として扱い、クラッシュ、DoS、情報漏えい、意図しないファイルアクセス、
誤った化学的安全判定を防ぐ。安全性と化学的妥当性は別の判定として記録する。

### Securityの完了基準

| 指標 | 必須条件 |
|---|---|
| Memory safety | `unsafe`を追加しない。panic、OOM、未定義動作を敵対的入力で再現不能にする |
| Resource safety | 入力サイズ、深さ、候補数、node数、時間、メモリを境界付きで拒否／停止する |
| Protocol safety | MCP/CLI/APIがmalformed、unknown、duplicate、巨大、順序不正の入力をfail-closedで処理する |
| Data safety | path、URL、archive、template、stock、evidenceの出所と許可範囲を検証する |
| Supply chain | dependency、workflow action、release artifactを固定・監査し、例外を期限付きで管理する |
| Detection | security regressionをCIで再現し、severity、affected surface、修正versionを追跡する |

### Phase S0 — Threat model and security contract

**目的:** 守る資産、攻撃者、信頼境界、許容する失敗を明文化する。

- CLI、Python、WASM、MCP、library、CI/releaseをsurface別に棚卸しする。
- attacker-controlled inputとtrusted local bundleを型・API・manifest上で区別する。
- malformed input、resource exhaustion、path traversal、秘密情報露出、依存汚染、
  validator confusionをthreat scenarioとして登録する。
- `security_case_id`、severity、再現入力hash、affected version、修正commitをsecurity
  regression recordへ統一する。

**Exit gate:** surfaceごとの脅威モデル、security owner、再現可能な最小fixture、
  release blocker定義が揃う。

### Phase S1 — Parser and boundary hardening

**目的:** 入力を解釈する最初の境界で、曖昧さとクラッシュを止める。

- SMILES/SMIRKS、JSON/JSONL、route export、stock、template、evidenceのサイズ・行数・
  nesting・文字コード・重複・未知フィールドを明示的に検証する。
- MCP JSON-RPCのrequest id、method、params、notification、error response、unknown tool、
  duplicate/extra fieldsを仕様化し、malformed lineを黙って捨てない。
- CLI/Python/WASM/MCPで同じ入力検証結果とエラー分類を返す。
- panicを外部入力から到達可能な境界でテストし、秘密情報・ローカルパス・stack traceを
  error outputへ漏らさない。

**Exit gate:** property/fuzz test、malformed corpus、巨大入力、UTF-8境界、JSON-RPC異常系で
  panic・silent fallback・機密情報露出がない。

### Phase S2 — Resource and algorithmic DoS resistance

**目的:** 正しい入力でも探索や化学処理を無限・過大に実行させない。

- input bytes、molecule atoms/bonds、template数、候補数、depth、beam、node、model call、
  route数、wall-clock、CPU、メモリの上限をsurfaceごとに定義する。
- cooperative cancellationを探索だけでなく、template loading、SMILES/SMARTS matching、
  forward replay、audit、MCP request lifecycleへ拡張する。
- timeout、budget exhausted、parse rejection、validation failureを別分類にし、途中結果を
  完了成功として返さない。
- adversarial molecule、high-branching template、duplicate storm、巨大route、slow inputの
  benchmarkを固定する。

**Exit gate:** 各上限が実測され、p95/p99・peak RSS・CPU time・termination reasonが記録される。
 上限超過時にprocess全体を巻き込まず、決定論的な安全な失敗になる。

### Phase S3 — File, bundle, and data provenance security

**目的:** template、stock、model、evidence、archiveを安全にロードする。

- user-supplied pathは許可されたroot、regular file、サイズ、symlink、canonical pathを
  検証し、path traversalと意図しない上書きを防ぐ。
- 外部bundleはcontent hash、schema version、producer、license、source revision、生成日時を
  manifestで検証し、欠損・改ざん・schema mismatchをfail-loudにする。
- URL fetch、圧縮入力、archive展開を導入する場合は、redirect、TLS、展開後サイズ、file count、
  archive traversal、cache isolationを先に設計する。不要ならnetwork-freeを維持する。
- 価格、hazard、yield、condition、evidenceを出所なしのtrusted factとして扱わない。

**Exit gate:** tampered、truncated、duplicate、wrong-version、symlink、traversal、zip-bomb
 fixtureが拒否され、manifestと実体のhashが一致しない限り推論を開始しない。

### Phase S4 — MCP, API, and multi-tenant isolation

**目的:** LLMや外部プロセスから呼ばれる入口を、誤用と連続攻撃に耐えるものにする。

- MCP tool schemaを実装側のtyped validationと同じ契約にし、巨大引数、未知引数、異常な
  timeout、連続request、stdout混入、stderr漏えいをテストする。
- request単位のresource budget、concurrency制限、cancellation、correlation id、監査ログを
  設計する。ログにはSMILESやtokenなどの機密入力を平文で残さない選択肢を持つ。
- Python/WASM/library APIでは、呼び出し元がpanic・thread・global state・filesystemを
  制御できないようにし、並行呼び出しと再入性を検証する。
- sandboxがない環境での「安全」を主張せず、必要権限と推奨隔離方法を明記する。

**Exit gate:** protocol fuzz、concurrency stress、cancellation replay、cross-surface
 differential testがpassし、1 requestの失敗が他requestやworkerへ波及しない。

### Phase S5 — Supply-chain and release security

**目的:** コード以外から攻撃される経路を減らし、利用者が成果物を検証できるようにする。

- `cargo audit`、`cargo deny`、license/source policy、duplicate dependency、Rust/Python/npm/
  WASM artifactをCIで定期検査する。
- GitHub Actionsのpermissionをjob単位で最小化し、third-party actionをmajor更新任せにせず、
  pin、変更監視、secret境界、artifact upload/downloadを監査する。
- release artifact、wheel、crate、WASM、model/template/stock bundleへchecksumとprovenanceを
  付け、再現可能なrelease smokeをpublish前に完了する。
- advisory、修正version、migration、backport判断を`SECURITY.md`の報告手順と接続する。

**Exit gate:** critical/high advisory、未承認license、未検証artifact、過剰権限workflow、
  publish前smoke failureがreleaseを止める。

### Phase S6 — Continuous adversarial verification and incident response

**目的:** 一度の監査で終わらず、修正が戻らないことを検証する。

- fuzz corpus、property tests、mutation tests、dependency drift、secret scan、SBOM、
  reproducibility replayを定期実行する。
- security fixごとに、攻撃入力、影響surface、root cause、回帰テスト、修正version、
  disclosure statusを記録する。
- 失敗を`invalid_input`、`resource_exhausted`、`security_rejected`、`not_evaluable`、
  `chemical_invalid`に分け、セキュリティ拒否を化学的失敗として隠さない。
- 半年ごとに外部レビューまたは独立攻撃演習を行い、未解決riskを期限付きで受容する。

**Exit gate:** release前のadversarial suiteがpassし、未解決のcritical/high riskに owner、
 deadline、公開判断がある。修正後のsecurity advisoryとregression evidenceが揃う。

## Dependency and delivery order

```text
Phase 0 ──┬── Phase 1 ── 2A ── 2B ──┬── 2C ── Phase 3A ── 3B ── 3C ──┬── 3D
           │                         │                                  │
           └── Phase 4 ──┘                                  ├── Phase 5 ── 6A ── 6B ── 6C ── 6D
                                                            └── 4,903-target comparison
S0 ── S1 ── S2 ──┬── S3 ── S4 ── S5 ── S6
                 └── security gates apply to every phase and every release.
SP1 ── SP2 ── SP3 ── SP4 ── SP5 ── SP6
  └── SP1/2 feed Phase 0/1/4, SP3/4 feed Phase 2/3, SP5/6 feed Phase 5/6/7.
Phase 7 is continuous; every phase has an artifact, gate, and release decision.
```

推奨順は、(1) Phase 0の現行master baseline、(2) Phase 1のcoverage/cost実験とPhase 4の
correctness修正、(3) coverage modeのCLI/Python出荷、(4) Phase 2Aのmodel adapter/OpenRetro
benchmark、(5) Phase 2Bのgeneralization、(6) Phase 3A〜3Cの探索基盤、(7) Phase 5のstock/制約、
(8) Phase 6A〜6CのCASP基盤、(9) 必要性が確認できたPhase 2C/3D/6Dである。大規模benchmark、モデル学習、default behavior変更は
同じPRに混ぜない。

## Explicit non-goals

- 「脆弱性ゼロ」を証拠なしに宣言すること。ここでの完了は、脅威モデル・境界・回帰suite・
  残余リスクを公開可能な形で管理できることを意味する。
- AiZynthFinderやASKCOSの画面・設定項目をそのまま複製すること。
- 未検証のyield、condition、success probabilityをrouteの説得力として表示すること。
- stockサイズ、GPU、検索時間、target splitが異なる数字を一つのleaderboardへ混ぜること。
- 候補生成の欠落をrerankerで解決したと主張すること。
- 「routeが見つからない」を「合成不能」と断定すること。

## Next implementation slices

直近の着手単位は次の順とする。

1. **Security S0:** surface inventory、threat model、resource budget、release blockerを
   `SECURITY.md`とrun manifestへ接続する。
2. **Security S1/S2:** MCP/CLI/Python/WASMの入力上限、typed validation、timeout/cancellation、
   malformed・巨大・高分岐入力のadversarial suiteを固定する。
3. **Current-master benchmark:** Phase 0 manifest、failure taxonomy、semantic hashを
   固定し、4,903-targetの再比較に使えるbaseline artifactを作る。
4. **Coverage-mode productization:** 既存のcooperative cancellationとStage 1/2 orchestratorを
   CLI/Pythonへ接続し、standard mode byte-compatibility、asset fail-loud、timeout分類を検証する。
5. **Correctness and coverage wave:** halide/carboxylation等の既知境界ケースを修正し、
   disjoint VALでcoverage gainとregression=0を測る。新しい条件モデルはこのgate後に着手する。
6. **Exploration contract:** proposer／inventory／search／scorer／traceを分離し、A*・beam・
   coverage modeを同じbenchmark harnessで比較できる状態にする。
7. **CASP foundation:** evidenceとfast-filterのcontractを先に出し、条件・収率モデルは
   evidence-linked retrievalとabstentionを通過した後に追加する。
8. **Generalization benchmark:** Chemformer／RetroGFN系を含むone-stepモデルを固定splitで
   比較し、OOD・round-trip・diversity・abstentionをroute coverageとは別に報告する。
9. **SynPlanner parity slice:** SP1の反応データmanifestとSP2のrule extraction safety gateを
   先に作り、policy/value・MCTS・route clusteringはその再現可能な入力の上に積み上げる。
10. **Union-gap closure:** G0のpaired failure atlasを生成し、G1の対象限定adaptive
    recoveryを実装・28件で測定済み。`recovery-beam-width` armは6/28回収、p95 wall-clock
    37.0秒、p95 RSS約74.4MiBだった。regression smoke 10件ではbaseline 6/10に対し
    recovery 8/10で、既存成功の減少は0件、timeout/crashも0件。回収対象を2回実行し、
    タイミング値を除いたJSON意味内容は一致した。次はVAL-200 armを測定してからG2/G3へ
    進む。AiZynthFinder-only 28件をdefault searchへ混ぜない。
11. **Overall performance:** O1のstrict validation差分分析、O2の共有recovery budget arm、
   O3の候補展開コスト削減、O4の複合paired gateの順で実装・測定する。coverageだけが
   改善しても、strict validityまたはp95が悪化した場合は勝利扱いにしない。
12. **Candidate augmentation first:** 追加候補を常時beamへ混在させず、native探索で未解決かつ
    正常終了した対象だけへ段階投入する。既存成功のregression=0を構造的に保証したうえで、
    candidate coverage・template被覆・stock差を改善し、同一条件のAiZynthFinder成功率超えを目標とする。

Candidate augmentationの初回gap-28検証では、AiZynthFinder-only対象を同一common-stock
unionで測定した。500-template nativeは3/28、5,000-template候補を直接生成fixtureとして
未解決対象だけへ投入しても3/28（regression=0、invalid=0）だった。一方、5,000-templateを
第2段のcoverage recoveryとして投入すると5/28（+2件、+7.1pp、regression=0、invalid=0、
timeout/crash=0）まで回収できた。単純な直接候補追加はroute回収に結びつかず、次は反応中心・
候補の化学的妥当性・downstream stock到達性を分離して改善する。native gapのstock差を
混ぜないよう、以後の正式測定はcommon-stock manifestを必須とする。

追加候補がnative beamを押し出す経路を残さないため、staged retryには`retro_generator_slots`
を導入した。native-onlyの上位beamを先に保持し、direct-generator候補は追加枠だけへ収容する。
既定値は0で既存検索の挙動を変えず、staged retryだけがbeam幅の5分の1（最小1）をopt-inする。
再ビルド済みbinaryで同じgap 28件を再測定した結果は3/28で、regression=0、invalid=0、
timeout/crash=0だった。安全な候補 admissionは確認できたが、現行の静的5,000-template
fixtureはdownstream stock到達性を改善しなかったため、AiZynthFinder超えは未達である。

その後、coverage tierを実際に有効化したfull staged recoveryを500→5,000→9,997 usable
templateで実施した。結果は7/28（5,000 tier比+2件、500 baseline比+4件）で、追加回収は
L1399/L371がcoverage tier、L274がelement-accounting、L370がbeam wideningとして監査上
分離できた。500 baselineの3件は全て維持し、regression=0、invalid=0、timeout/crash=0。
一方でp95 elapsed 54.67秒、p95 RSS 353.3MiBまで増えるため、coverage tierは既定化せず、
入力・tier数・予算をmanifest固定したopt-in段階投入として扱う。

現行の全テンプレート入力（`templates_10000.smi`をCLIが9,974 rulesとして読込）から同じgap 28件の
直接生成候補を再生成し、native beam保持付きの段階投入を再測定した。候補poolは2,676行、
zero-candidateは0/28、生成時間p95は0.025秒/targetだったが、strict coverageは3/28のままで
追加回収はなかった。baseline回帰0、invalid=0、timeout/crash=0であり、候補数不足ではなく
反応中心の適用可否、候補の化学的妥当性、または後続stock到達性がボトルネックであることを示す。
次の実装はこの3要因を候補単位で分離するdiagnosticと、positive候補だけを段階投入する補完策に
限定する。直接候補追加のdefault化やAiZynthFinder超えの主張はまだ行わない。

段階投入の追加枠は`--retro-generator-slots N`で明示指定できるようにした。上限は1,000件
（direct-generatorの1回あたり提案上限）で、未指定時は0のまま既存挙動を維持する。これにより
native beam保持、投入枠、候補pool、stock frontierの各条件をmanifestへ固定して比較できる。

追加候補の入口では、direct-generator proposalを既存のroute-integrity境界へ通し、構造的に
strict routeになれない候補（例: target元素を満たさない候補）をfrontierへ入れないようにした。
この事前検証はnative候補へは適用せず、追加候補だけの探索ノイズを減らす。invalid候補の除外と
既存のdirect-generator CLI契約をテストで固定した。

さらに同poolをcommon-stock unionへexact canonical-SMILES照合する診断を追加した。2,676候補の
うち全前駆体がstockに入る候補は3件・3/28対象だけで、残り25/28対象は少なくとも1段の
downstream searchを必要とする。これはstock差そのものではなく、直接候補からstock終端へ至る
探索・適用の問題を示す結果であり、`scripts/diagnose_candidate_stock_frontier.py`と診断JSONを
証跡として保存する。部分stockヒットは成功とは数えず、構造妥当性・候補適用・stock終端を別々に
評価する方針を維持する。

また、static generator変換時にpoolの`best_upstream_rank`を行順で上書きしていたため、元の
template順位を`source_rank`として保持するよう修正した。最新release binaryで再測定しても
strict coverageは3/28のままで、regression=0、invalid=0、timeout/crash=0だった。これは
選択順の復元自体は正しいが、現在のgapでは直接候補だけでstock終端へ届かないことを示す。
次はdirect candidateのdownstream展開側を測定・改善する。

root候補から抽出した2,617中間体へ9,974-template候補を生成し、stockヒット数、template順位の
順で各32候補にbounded selectionしたroot＋intermediate fixtureも検証した。gap 28件ではstrict
coverageは3/28のままで、regression=0、invalid=0、timeout/crash=0だった。p95 RSSは約155MiB
まで増えたため、候補供給範囲だけを広げる方式は採用せず、次は反応中心適用とdownstreamの
候補選択を一体で評価する。

その後、中間体ごとにstockヒット数、次にtemplate順位で上位32候補を選び、direct-generatorの
`source_rank`を0.02以内のordering-only補正として接続した。最新release binaryで同じgap 28件を
再測定した結果、strict coverageは3/28から5/28へ増加し、L1399/L370を追加回収した。500-template
baselineの3件はすべて維持し、regression=0、strict invalid=0、timeout/crash=0だった。p95 RSSは
約161MiBで、L370の`stereo_center_count_mismatch`/`charge_imbalance`は情報警告として別記録した。
これは段階投入、候補選択、順位伝播を組み合わせた最初のpositive recall改善であるが、gap 28件の
限定結果であり、AiZynthFinder全体を上回った証拠ではない。

候補上限のpaired確認では、stock優先32候補が5/28だったのに対し、48候補は4/28へ低下し、
p95 RSSは約208MiBへ増えた。baseline回帰、invalid、timeout、crashは0だったが、追加候補を
増やすだけでcoverageが単調増加しないことを確認したため、32候補を現行のbounded sweet spot
として固定し、48候補は採用しない。

direct-generatorの実行時に各proposalをexact canonical-SMILES stock membershipで照合し、
stock充足率を検索コストへ混ぜる案もpaired測定した。しかしgap 28件でstrict coverageが
従来5/28から4/28へ悪化したため、実装は撤回した。stock近傍情報は診断・候補選択層に留め、
化学コストと混ぜない。したがってstock差は候補の近さだけでなく、反応中心の適用可否と
downstream positive recallが本質的な次課題である。

候補被覆の追加対策として、`bond_index`が空、またはindexed rule適用後に候補がゼロとなった
場合だけ、全rule setへ再試行するfallbackを追加した。候補が得られる非空index結果は従来どおり
保持し、空結果とzero-proposalの回数を別々に診断へ記録する。これにより粗いbond signatureに
よる取りこぼしを回収できるが、現在の主測定はbond indexを使わないため、gap 28件のcoverage
改善とはまだ結びつけていない。

比較ハーネスにも`--bond-index`を接続し、検索診断のfallback回数を結果へ保存できるようにした。
fixed gap 8件のpaired smokeではfallbackは空結果・zero-proposalとも0件で、route foundは1/8、
invalid=0、timeout/crash=0だった。少なくともこのサンプルでは、bond indexの取りこぼしは
AiZynthFinder-only gapの主因ではない。

staged direct-generator（中間体stock優先32候補・追加slot20）と`bond_index`の併用もfixed gap 28件で
測定した。strict coverageは5/28で従来最良を維持し、baseline回帰0、invalid=0、timeout/crash=0、
empty/no-proposal fallbackはいずれも0だった。p95 elapsedは約4.84秒、p95 RSSは約155MiBで、
bond-indexを足すだけの追加回収は確認できなかった。

低recallの取りこぼしをさらに限定的に扱うため、bond-indexが候補を1件以下しか返さない
cache-missでは全rule setを再照合し、indexed poolへ追加して通常のcandidate-level mergeへ
渡すfallbackを実装した。bond-index無効時と2件以上の候補が得られた通常経路は不変である。
fixed gap 28件をdirect-generator slot20と併用して再測定した結果は5/28で従来最良を維持し、
regression=0、invalid=0、timeout/crash=0、p95 elapsed約4.55秒、p95 RSS約152MiBだった。
現時点でcoverage増はないためdefault化せず、bond-index opt-inのrecall guardとして維持する。

direct-generator追加枠内で即時stock終端候補を強制予約する案もpaired測定したが、fixed gap 28件の
strict coverageは5/28のままで、p95 elapsedは約5.16秒へ悪化した。このため予約は撤回し、追加枠の
source-rank順とnative beam保持を維持する。stock情報を検索コストへ混ぜる案も既に負の結果があるため、
stockは候補選択・診断の入力として扱い、化学コストの代替にはしない。

全候補fixtureに対して1段stock-lookahead（各前駆体の次段候補に全stock終端があるか）を使う
bounded selectorも実装した。しかしfixed gap 28件ではstrict coverageが4/28となり、既存の
stock優先32候補の5/28を下回った。候補poolの局所的な先読みを検索順位へ混ぜると、有望な
alternative routeを押し出すため、検索fixtureへの採用は見送る。selectorは候補選択実験と
negative-result再現のために残し、native beam保持とsource-rank順を優先する。

recovery modeでは、従来configにgeneratorが残ったままbaselineを実行すると、追加候補が段階開始前
からnative frontierへ混在し得た。この境界を修正し、recovery baselineをgeneratorなしのnative passへ
固定した。baselineが正常終了してrouteなしの場合だけ、native beamを先に保持するdirect-generator
stageを実行し、その後にdepth/beam/coverage retryへ進む。これによりstandard modeだけでなくrecovery
modeでも、既存native成功をgeneratorで置換しない契約を明示した。

この段階化をcoverage tierと組み合わせ、fixed gap 28をnative baseline → direct-generator追加枠
→ depth/beam → 5,000/9,974-template coverage tierの順で測定した。strict routeは7/28で、内訳は
baseline 3、direct-generator 2（L1399/L370）、element-accounting 1（L274）、coverage 1（L371）
だった。baseline成功は全て保持しregression=0、invalid=0、timeout/crash=0、route treeは7/7
parseable。p95 elapsedは約26.6秒、p95 RSSは約408MiBであるため、coverage改善の候補としては
有望だが、常時defaultではなく明示的なdeep recovery / benchmark armに限定する。

明示的に`retro_generator_slots`を指定した場合の追加候補段階も実装した。初回generator stageが
正常終了してrouteなしの場合だけ、追加枠を最大2倍（既存のproposal上限まで）に広げた
`retro_generator_widened` stageを実行する。native beamは初回・拡張後の両方で先に保持され、
slots未指定時は従来どおり単一generator stage、generator未設定時はnative-onlyのままである。
fixture testでbaseline → generator → widenedの遷移と既存候補非置換を確認した。これは候補の
後順位を段階投入するための明示的deep recovery拡張であり、default searchへは昇格させない。

全中間体候補を含む約112MiBのhash-pinned artifactも、専用128MiB入力上限で評価可能にした。
しかしfixed gap先頭8件ではstrict routeが2/8に留まり、stock32候補fixtureからの増加はなく、
p95 RSSは約606MiBまで上昇した。したがって全artifact投入は候補被覆の研究診断に限定し、
通常運用・正式benchmarkではbounded stock32 fixtureとnative beam保持を優先する。

比較ハーネスにも`--retro-generator-slots`を接続し、実binaryへ渡した追加枠をconfiguration_idへ
含めるようにした。これにより、native beam保持条件とdirect-generator投入容量を、測定manifest
だけで再現・識別できる。

残存gapの一段候補と下流欠落を混同しないため、AiZynthFinderのroute edgeをRENKINのcanonical
identityへ変換してfull candidate poolと照合した。残存21件は正解一段候補を21/21で保持していたが、
代表L1006では競合経路の二段目以降にN-Boc precursor生成が必要で、RENKINの既存
`boc_deprotection_retro`だけではその方向を供給できなかった。既存ルールの意味と成功経路を変えず、
neutralな1/2級aliphatic amineだけを対象にした補完`boc_protection_retro`を追加した。構造・元素差分の
単体テストとactive-rule regressionは通過したが、L1006のstrict routeはこの補完だけでは未回収である。
候補生成のpositive recallは改善したものの、正式成功率の改善とは扱わず、下流stock到達性の調査を継続する。

競合route edgeと候補poolの照合は、再現可能な`diagnose_competitor_route_coverage.py`へ固定した。
canonicalizerを一括利用し、target候補の有無とprecursor multiset完全一致を別々に記録するため、
「templateが適用されない」「中間体が候補poolへ投入されていない」「候補はあるが下流stockへ届かない」
を混同しない。固定gap28の現行full poolでは143 edge中68 edgeにtarget候補があり、完全一致は53、
欠落targetは75だった。欠落75 targetをfull 9,974-templateで直接生成すると75/75がnon-zero
（3,928 rows、p95 0.0108秒/target）だったため、現時点ではtemplate適用不能を主因と断定しない。
この結果は候補被覆の診断値であり、競合routeの化学的妥当性やRENKINの
strict成功率を直接意味しない。次は欠落targetを反応family・中間体投入漏れ・候補選択・stock frontier
に分解し、既存候補を押し出さない限定的な補完だけを採用する。

候補の中間体投入は`expand_candidate_frontier.py`で段階化する。各roundは前roundのprecursorを
boundedに次の`renkin-pool-gen`へ渡し、候補行は全roundでunionするため、追加候補が既存候補を
押し出さない。compiled stockを指定した場合はstock-terminal moleculeを次段入力から除外する。
これは外部モデルやvalidation labelsに依存しないfixture生成基盤であり、round別の入力数・候補数・
SHA-256を保存して再現性を確保する。検索本体へ既定投入する前に、固定cohortでregression=0と
positive recallをpaired検証する。

投入上限32と256を同一条件で比較した結果、どちらも7/28で既存成功を全保持したが、256は
27,541候補行まで増やしても新規成功を生まず、p95 total elapsedは約95.0秒（32は約87.1秒）へ
悪化した。したがって候補量の単純な増加は採用せず、stock-terminalを除いた中間体の選択品質、
候補の下流到達性、必要なら反応family別のquotaを測定対象にする。

親候補のstock hit ratio・hit数・source rankを使うstock-aware frontier選択も追加したが、
固定gap28の2-round測定は5,333候補行・7/28で、非stock-aware版から成功率は変わらなかった。
p95 total elapsedは約102.0秒だったため、stock近接度だけをproxyにする方針は採用しない。次段では
候補targetごとの子候補数、子候補のstock到達深さ、候補グラフのdead-end率を明示的に計算し、
下流到達性を直接使った選択をpaired評価する。

candidate-graph selectorのfull VAL-200では、同一run内のnative first attemptが133/200、
recovery resultが143/200となり、10件を追加回収した。completed native successのregression=0、
strict validated=143/200、parseable=143/143、crash=0、timeout=5/200だった。timeoutは成功に
含めず、AiZynthFinderの123/200は条件差（model、stock artifact、timeout/search semantics）が
あるため参考値に留める。詳細は`data/comparison/formal_v1.0.3_candidate_20260909/graph_selector_val200/report.md`。
正式なsame-condition superiority claimは、同一stock identity・同一budgetでのpaired再測定まで保留する。
既存のAiZynthFinder shared-stock HDF5/configは確認できたが、再測定開始時点でDocker daemonが応答
しなかったため、同一条件armは未実行である。これはRENKINの測定失敗ではなく環境状態として記録し、
daemon復旧後に再開する。

このため、full candidate poolのtarget→child candidate graphを利用する
`select_retro_generator_graph.py`を追加した。root targetは全候補を保持し、中間targetだけを
child count、child stock-terminal count、child stock ratio、source rankでbounded selectionする。
これは候補行を削除して既存routeを変える機構ではなく、追加投入用artifactの中間target数だけを
制限する。固定gap28でstock-aware policyと同じrecovery条件のpaired測定を行い、7/28から
9/28へ改善した。新規回収はL274/L370、既存7件のregressionは0、invalid=0、timeout/crash=0、
route tree parseable=9/9だった。p95 total elapsedは約101.5秒であるため、速度との交換条件を
明記したopt-in候補段階として扱い、全VALへ適用する前に回収対象の再現性を確認する。
新規L274/L370は同じartifact・同じrecovery条件で再実行し、2/2 route_found、strict validated
2/2、parseable 2/2、invalid=0、timeout/crash=0を再現した。次はこのselectorをfull VALへ
適用し、native baselineとの200件回帰を確定する。

selectorの入力境界も補強した。`select_retro_generator_graph.py`は従来のJSONL candidate
poolに加えて、static generatorのhash-pinned `proposals` artifactを直接読み込める。
artifact出力では元candidate objectとprovenanceを保持し、root targetは全候補を維持したまま
中間targetだけをbounded selectionする。これにより、候補生成→選択→static generator変換の
間で手動変換による候補・証跡の欠落が起きない。入力変換、provenance保持、root非置換を4テストで
固定した。これは成功率改善の測定ではなく、段階投入を安全に再実行するための基盤改善である。

さらに`diagnose_competitor_route_coverage.py`へ`--missing-targets-output`を追加し、候補poolに
targetが存在しない競合edgeを、決定論的な`renkin-pool-gen` groupへ変換できるようにした。VAL-200
の310 missing targetをfull 9,974-templateで直接生成したところ、310/310がnon-zeroとなり、
20,808 candidate rowsを得た。既存poolへunionした診断では、競合edgeのtarget候補被覆が68/378
から378/378、precursor multiset完全一致が52/378から265/378へ増加した。したがってこのcohort
ではtemplate適用不能より、生成済み候補の中間体投入・下流選択・正規化差が主要な次課題である。
生成artifactと前後診断は`data/comparison/formal_v1.0.3_candidate_20260909/competitor_missing_val200/`
へ保存した。ただし、これは候補被覆の改善でありroute成功率ではないため、native candidateを
押し出さないstaged recoveryでregression=0を確認するまでdefault検索へは接続しない。

このunion artifactを実際のVAL-200 staged recoveryへ投入した。native first attemptは133/200、
最終route-foundは143/200で、10件を回収しつつregression=0、strict validated=143/200、
parseable=143/143、timeout=5、crash=0だった。しかし既存graph selectorの143/200から増加は
なかった。full templateで候補を生成できても成功率が増えないため、候補生成不能ではなく、
中間候補の下流選択、exact normalization、stock-reachable routeの優先順位が次のボトルネックで
ある。runとartifactは`data/comparison/formal_v1.0.3_candidate_20260909/candidate_expansion_val200/`
に保存し、候補追加をdefaultへ昇格する判断は保留する。

非置換契約を測定者の手計算に依存させないため、`verify_non_displacing_recovery.py`を追加した。
各rowのrecovery attempts[0]をnative baseline、rowの最終`route_found`をselected resultとして
比較し、native成功の消失をregressionとして検出する。candidate expansion VAL-200ではbaseline
133、final 143、recovered 10、regression 0、timeout 5、crash 0を機械検証した。今後の
候補段階投入はこのゲートを必須とし、候補被覆の増加だけではdefault化しない。

下流到達性を直接使う別案として、selectorへopt-inのbounded multi-hop stock lookaheadを追加した。
candidate graphを最大指定深さまで再帰的に評価し、stockへ到達可能なcandidateをintermediate
target内で優先する。循環は保守的に打ち切り、root候補は全保持する。fixed gap28では8/28、
regression=0、invalid=0、timeout/crash=0だったが、既存graph selectorの9/28を下回ったため
default化しない。stock近接や局所lookaheadだけでは、反応経路の多様性を保持するより有利とは
限らないことを確認した。測定artifactは`data/comparison/formal_v1.0.3_candidate_20260909/lookahead_gap28/`
に保存した。

stock lookaheadの負の結果を踏まえ、stockを順位の主軸にせず候補の多様性を保つopt-in selectorを
追加した。各intermediate targetで、候補の前駆体数とsource-rank帯をbucketとして扱い、各bucket
から最良候補を先に採用してから残りを通常順位で埋める。これはreaction familyそのものではない
label-free proxyであり、root targetの候補は全保持する。実artifactではroot 134 targetを保持し、
中間2,664 targetを各16候補へbounded selectionできた。fixed gap28のpaired測定では7/28
(baseline 5/28、追加2件)、regression=0、invalid=0、timeout/crash=0だったが、既存graph
selectorの9/28を下回った。したがってdefaultにはせず、候補多様性だけでは下流到達性を十分に
改善しないnegative resultとして記録する。

stock到達可能なpartial overlap 24件を中間targetとして無制限保持する診断armも測定した。
固定gap28ではbaseline 5/28、最終7/28、追加2件、regression=0、invalid=0、timeout/crash=0
だったが、既存graph selectorの9/28を超えず、p95 total elapsedは90.24秒だった。このため
本番policyには採用せず、次は24件をreaction family・representation別に分類して、family別の
非押し出し補完をpaired評価する。

候補が探索入口で失われる箇所を特定するため、direct-generatorの供給数・採用数・integrity拒否・
SMILES parse拒否・自己同一候補拒否を`SearchStats::crowd_out`へ記録する計装を追加した。これは
探索順序や候補 admissionを変更しない。次のpaired実験ではこの計装を有効にして、candidate
lossと下流beam/stock lossを分離する。
固定gap28へ適用した結果、generator stageは23,685候補を供給し23,407を採用、278をintegrity
拒否、parse拒否0だった。最終成功は7/28であり、candidate入口より下流探索・stock終端が制限要因
であることを確認した。artifactは`data/comparison/formal_v1.0.3_candidate_20260909/instrumented_gap28/`
に保存した。
generator候補を後続のelement/beam/depth retryへ引き継ぐ変更も実装した。固定gap28では7/28
(baseline 5/28、追加2件、regression=0)のままで、p95 total elapsedは102.34秒へ増加した。
候補が後続budgetへ届くことは確認できたが、成功率はgraph selectorを超えないためopt-in実験に
留め、default化しない。artifactは`data/comparison/formal_v1.0.3_candidate_20260909/propagated_generator_gap28/`
に保存した。

候補union後に残った113件のexact precursor mismatchについて、
`diagnose_candidate_mismatch.py`で候補pool内の共通precursor数を決定論的に分類した。partial
overlapは42件、共通precursorのないno overlapは71件だった。この診断はcandidate coverageの
形状を示すだけで、化学的同値性やstock到達性を推定しない。次はpartial overlap 42件を起点に、
正規化表現、反応family、下流stock reachabilityを別々に計測し、非押し出し補完をpaired評価する。
missing expected precursorのstock membershipも分類し、partial overlap 42件のうち28件は
欠落precursor自体がstockに存在し、14件は下流探索が必要だった。次はこの2群を分離して評価する。

同じ診断へcompiled canonical stockの保守的なgraph reachabilityを追加した。113 mismatchのうち
49件（partial overlap 24、no overlap 25）は候補artifact上でstockへ到達可能、64件は到達不能
だった。これは候補の化学的同値性や検索でのroute成功を意味しないが、まずstock到達可能な
partial overlap 24件を、既存成功を保持する非押し出し補完の評価対象とする根拠になる。

53.5acの診断入口として`scripts/diagnose_partial_overlap.py`を追加した。既存のJSON artifactと
candidate-pool JSONLを受け付け、RDKit canonical／stereo-stripped表現差、非断定的な
`reaction_family_proxy`、欠落前駆体のdirect-stock／downstream-or-unavailableを入力hash付き
JSONへ出力する。これはheuristic diagnostic onlyであり、route validity、検索順位、候補集合を
変更しない。実VAL-200のcoverage atlasと候補artifactを同一cohortで再計算したところ、
partial overlap 42件を検出した（amide_like 9、ester_like 2、sulfonamide_like 4、
carbonyl_oxygen_like 4、other_or_unknown 23）。欠落前駆体の表現判定はcanonical 3、
stereo-only 2、not-equivalent 32、候補unparseable/非同値8、stock directは1、
downstream-or-unavailableは44だった。これは拡張候補artifact上の診断であり、化学的同値性や
route成功を意味しない。証跡は`data/comparison/formal_v1.0.3_candidate_20260909/
candidate_expansion_val200/partial_overlap_diagnostic_recomputed.json`に保存し、24件の
stock-reachable subsetに対するfamily別非押し出しpaired評価を次段階で実施する。

53.5acの次段階として、24 edge（21 unique target）を`reaction_family_proxy`別のfixtureへ
分割する`prepare_partial_overlap_family_cohorts.py`を追加した。amide_like 5、
carbonyl_oxygen_like 1、ester_like 1、sulfonamide_like 2、other_or_unknown 12の
unique targetを同一sample contract・同一stock/template設定でbaseline測定し、合計19/21
(90.5%)、invalid・timeout・crash 0を得た。これはfamily別の問題規模を比較するbaselineであり、
既存候補を保持した非押し出し補完のpaired結果ではない。中間targetに対するdirect-generator
recoveryは今回のbaseline測定では発動しておらず、証跡は`data/comparison/formal_v1.0.3_candidate_20260909/
candidate_expansion_val200/`配下に保存した。

同じ21 unique targetへRecovery監査を再実行した。全21件が正常完了し、route_foundは3/21、
direct-generatorは19/21件で起動、5,043 proposal中4,980件を追加枠へadmitしたが、generator
stage単独の新規回収は0件だった。回収3件はbaseline/beam/depthの後段で、integrity拒否63、
parse拒否・自己同一候補拒否0。これは候補を押し出さない段階投入と監査計装の確認であり、
standard modeのbaseline 19/21との差を同一実行内で比較した性能paired efficacyではない。
ただし、Recovery内の非押し出し検証はbaseline raw成功1→最終3、回収2、regression=0、
timeout/crash=0で通過した。generator stage単独の新規回収は0件だったため、53.5acは
候補供給の保護とnegative resultの確定まで完了し、下流探索・stock終端の改善を次課題とする。
結果はfamily_samples配下の`*_recovery_result.jsonl`、`*_recovery_aggregate.json`、および
`partial_overlap_recovery_non_displacing_verification.json`に保存した。

O1の初回差分分類を実施した。VAL-200 recoveryではstrict validator-confirmedが127/200
(63.5%)、未評価8件は全て深さ0の直接購入であり、実行失敗や化学的な棄却ではない。
共通警告は`stereo_center_count_mismatch`のみで、情報警告として扱われる。したがって
反応validator-confirmed率では非評価のまま保持する一方、stock-terminal複合指標では
「共通validator合格、またはstock identityを確認したdepth=0」と契約を明示して直接購入を
正当な成功として数える。立体情報を無視して検証を緩めるものではない。分類結果は
`data/comparison/formal_v1.0.2/o1_recovery_quality.json`、再生成スクリプトは
`scripts/analyze_recovery_quality.py`に固定した。回復stage別の内訳はbaseline 121、
depth 66、element_accounting 7、beam_width 6である。

49.6の独立cohort初回測定として、VAL由来の固定disjoint-200
(`data/phase_b1_frontier/val_sample_disjoint_200.jsonl`)へ現行fast profileを適用した。
入力・stock・templateのhash付きmanifestを生成し、route_found/strict validatedは21/200
(10.5%)、p95 total elapsed 5.701秒、p95 RSS 44.7 MiB、invalid・timeout・crash 0だった。
これは200件の固定cohortに対する初回generalization evidenceであり、再実行で同一route_found
件数と0失敗を確認するまで単独では再現性の主張に使わない。初回証跡は
`data/comparison/search_profiles_disjoint_200_20260911/`、再実行証跡は
`data/comparison/search_profiles_disjoint_200_20260911_rerun/`に保存した。両runの入力・stock・
template hashとconfiguration IDは一致し、route_found/strict validatedはともに21/200
(10.5%)、invalid・timeout・crashもともに0だった。これはこのdisjoint-200 cohortに限る
再現性・一般化の証拠であり、外部corpusや競合優位性の主張ではない。

48.20の残存18件は、既存の凍結holdout証跡を再監査した。18/18 process completed、
timeout・crash・invalid outputは0、新規valid routeは0だった。完成候補10,433件は全て
`unaccounted_target_element`で棄却されており、実行環境の失敗ではなく、TRAIN-only
abstractionが生成する候補の元素accounting境界が主因と診断した。追加corpusや同holdout
への再調整はせず、原因診断完了・coverage解決未完了として扱う。

次のO1実装として、比較adapterに`--max-routes`と`--route-selection strict_validated`
を追加した。既存のrank-1 armは変更せず、明示した実験armだけが複数候補を受け取り、
同じroute-tree／reaction-step／element-accounting validatorを通過した最初の候補を
選ぶ。これはstrict validity改善の検証用であり、候補数増加による速度悪化を含めて
別configuration_idで測定する。

VAL-200先頭10件のsmokeでは`max-routes=3`・`strict_validated`のstrict成功は6/10で、
選択された候補は全てrank-1だった。現段階ではstrict改善効果は確認できず、p95 wall-clock
は26.69秒、p95 RSSは約65.8MiBだった。このため候補数を無条件に増やすのは採用せず、
次は候補がrank-1 validatorで落ちる対象だけを対象にした条件付きportfolio armと、
候補展開コスト削減を優先する。

条件付きarmとして`strict_on_rank1_failure`も追加した。rank-1が同じvalidatorを通過
した場合は後続候補を評価せず、失敗時だけ返却済み候補を走査する設計である。
3件smokeではroute_found 1/3、strict 1/3、選択indexは0で、crash/timeoutは0だった。
候補選択の接続と既存契約の維持は確認できたが、改善効果の主張には不十分なため、
正式gateには算入しない。

O3の準備として`renkin-bench --include-routes`を追加した。既存の共有プロセス実行で
得たbest routeを任意にJSONへ含められるため、stock/templateを対象ごとに再ロードせず、
後段のstrict validatorや比較adapterへ同じrouteを渡せる。デフォルト出力と既存の
benchmark指標は変更しない。次はこの出力を読むbatch比較経路を追加する。

`scripts/compare_renkin_batch.py`でこの経路を比較schemaへ接続した。3件smokeでは
route_found 1/3、strict validator-confirmed 1/3、crash/timeout 0で、route payloadの
再検証も通過した。batch測定はプロセス内共有ロードの効果を見るためのものであり、
isolated RSSを持たないため正式なlatency/RSS gateとは分離する。
release版でVAL先頭10件を測定したところ、route_foundは6/10でper-target armと一致した。
対象内計測時間の合計は21.27秒で、同じ10件のper-target armの198.27秒に対して約9.3倍
短縮した。これはプロセス再起動・stock/template再ロードの削減を示すbatch throughput
証拠であり、per-target p95の勝利や競合比較の証拠には外挿しない。
続けてVAL-200全件を実行し、route_found 121/200 (60.5%)、strict 113/200 (56.5%)を
確認した。対象内計測時間は合計439.1秒、p50 0.69秒、p95 9.43秒だった。これは
baseline d5/b100の再現値であり、recovery 135/200 (67.5%)のcoverage改善とは別物である。
batch経路は速度基盤として有望だが、次はrecovery cascadeを同一プロセスへ移す必要がある。
`renkin-bench --search-mode recovery --recovery-depth 6 --recovery-beam-width 200`と
`scripts/compare_renkin_batch.py`も接続し、3件smokeでroute_found 2/3、strict 2/3、
crash/timeout 0を確認した。次はこのbatch recoveryをVAL-200で測定し、既存の
per-target recovery 135/200との一致と総時間を確認する。
無期限のVAL-200 recovery batchは、対象ごとの深い探索が長時間化するため中断した。
batch adapterに外部process timeoutを追加し、共有recovery予算30秒のVAL先頭10件を再測定。
route_found 8/10、strict 8/10、合計27.04秒、p95 4.27秒、crash/timeout 0だった。
今後の正式batch armは必ずrecovery予算と外部process timeoutをmanifestへ固定する。
batch結果へ`recovery_audit`を追加し、baseline／element_accounting／beam_width／depthの
各stage、termination、nodes、elapsedを失わず比較行へ渡せるようにした。これで次回の
難ケース再測定では、coverage一致だけでなくstage選択の一致も検証できる。
未発見routeでもauditを失わないようadapter配線を修正し、難ケース3件でselected_stage
とattempt数の保存を確認した。route_found 0/3だったが、全件のauditが保持され、stage
選択を後から監査できる状態になった。
G1難ケース28件を同じbounded recovery設定で再測定し、route_found/strictとも6/28で
per-target recoveryと対象IDが完全一致した。合計213.0秒、p50 5.24秒、p95 17.49秒、
最大24.13秒、crash/timeout 0だった。共有プロセス化による結果再現性は確認できたため、
次はbatch結果のstage監査を追加し、full VALへ進む前にrecovery予算の妥当性を評価する。
追加のO3候補としてbatch adapterに`--bond-index`を接続した。VAL先頭10件ではcoverage
6/10、strict 6/10で非indexed armと一致した。合計時間は22.78秒、p50 1.34秒、p95
2.90秒で、p95の外れ値は減った一方、合計時間は非indexedの21.27秒より約7%増えた。
現段階では無条件default化せず、対象分布を増やして再測定する。
O3の並列化armとして`scripts/compare_renkin_parallel_batch.py`を追加した。workerごとに
batch processを持つためstock/templateを複製する構成だが、対象順序と結果を再整列し、
strict判定を維持する。2 worker・VAL先頭10件ではroute/strictとも6/10、成功IDもserial
batchと一致したが、wall-clockは約58秒でserial batch（約47秒）より遅かった。stock複製と
起動競合のコストが支配的なためdefault化せず、共有単一processの改善を優先する。
並列化後の次の判断材料として`analyze_batch_cost.py`を追加した。recovery VAL先頭10件の
コスト分類はbaseline 6、beam_width 1、depth 2、element_accounting 1で、試行termination
は全てcompletedだった。次の改善はこのstage圧力を使った条件付き探索削減に限定し、
単なるworker数増加は採用しない。
探索プロファイル用に`--timing-diagnostics`をbenchmarkへ追加し、batch adapterにも接続
した。指定時だけretro展開時間、dedup前候補数、cross-template dedup後候補数を出力し、
通常のbenchmark出力と性能には影響しない。3件smokeで、例えばL3595は展開115.3ms・
候補1817→1532と確認できたため、次はこの高分岐対象を絞ったdedup実装を検討する。
同一親ノードのcross-template重複を実際に除去する`cross_template_dedup`をSearchConfigへ
追加し、benchmarkでは`--cross-template-dedup`でopt-in可能にした。defaultはOFFで、
template provenanceを保持する従来挙動を壊さない。既存dedup検出テストとbenchmark
binaryのcompileを通過した。G1固定サンプル10件ではON時に候補が最大7043→5912、
2656→1961など減少したが、同じサンプルの別実行でroute/strictはON 2/10、OFF 0/10と
探索順序の揺らぎが出ており、候補削減だけでは改善根拠にならない。provenanceを落とす
この方式は正式経路に採用せず、既定OFFの実験フラグとして保持する。次は候補の
provenanceを保持したままbeam crowd-outを緩和する、制約付き多様性選抜を優先する。
既存の`BeamDiversityPolicy::Active`をbenchmark CLIにも接続し、`--beam-diversity-policy`
と`--beam-diversity-slots`で再現可能な比較ができるようにした。予約10枠のG1固定サンプル
10件ではstrict 1/10となり、同一サンプルのOFF再測定0/10からの改善は1件だけだった。
実行時間合計は約10.95秒で、coverage改善を主張できる規模ではない。したがってActiveも
default化せず、次は予約枠を固定せず、beam圧力・family数に応じて予約数を制約する
adaptive diversity selectionを測定する。
recoveryの`PreparedRuleSet`とbaseline用bond indexを対象ごとに再構築していたため、
`RecoveryContext`を追加し、batch内でimmutableな準備済み資産を共有する経路へ変更した。
既存の単発APIはwrapperとして維持し、標準modeとcoverage tierのindex対応は変更していない。
次はこの変更をrelease batchで再測定し、初期化時間削減を確認する。
release版bounded recovery VAL先頭10件ではroute/strictとも8/10、成功IDも一致した。
合計29.13秒、p95 4.33秒で、旧context armの27.04秒より約7.7%遅かった。準備資産の
共有は対象間の再構築を除く正しい設計だが、この小標本では測定揺らぎまたは検索コストが
支配的で、速度向上は未証明として扱う。
同一baseline rule setでrootの`matched_templates == 0`となる対象は、baselineのbeam／
depth recoveryでは新しい探索辺を作れないため、これらの無効なretryをスキップする条件を
追加した。coverage tierは別rule setとして引き続き実行可能で、既存の標準mode挙動は
変更しない。`cargo check`とrecovery CLI回帰テストで確認した。
release版VAL先頭10件で再測定したところ、route/strictとも8/10、成功IDも従来と一致した。
一方、合計時間は30.30秒で、変更前の27.04秒より約12%遅かった。templateゼロ対象の
削減効果はこの小標本では他対象の揺らぎに埋もれたため、default性能向上としては
主張せず、条件付きskipの正しさのみ維持する。
recovery・bond-index併用をG1難ケース28件で再測定した結果、route_found/strictとも6/28で
非indexed armと完全一致した。合計224.32秒（非indexed 213.01秒）、p95 20.92秒
（非indexed 20.77秒）で、速度改善はなく約5.3%遅かった。したがってbond-index共有は
重複構築を除く内部改善として残すが、recovery armのdefault有効化は見送る。
recovery各stageで同じbaseline rule setの`TemplateBondIndex`を再構築していたため、
recovery開始時に一度だけ構築してstage間で共有する経路を追加した。coverage tierは
rule indexの対応が異なるため従来どおり別扱いとし、誤ったindex再利用を避けている。
recovery・bond-index併用のVAL先頭10件ではroute_found/strictとも8/10、合計26.98秒、
p50 2.05秒、p95 4.30秒だった。bond-indexなしのbounded arm（8/10、27.04秒、p95
4.27秒）とほぼ同等で、現サンプルでは有意な改善は確認できない。共有化は再構築の
重複を除く正しい実装として維持し、速度効果の主張は保留する。
G1難ケース28件のstandard armではbond-index有効時にroute_found 0/28、合計74.34秒、
p50 2.08秒、p95 7.05秒だった。recovery対象をstandard結果だけで評価するarmでは
coverageを回収できないため、bond-index単独のdefault化は見送り、recoveryとの互換性を
含む別設計として扱う。

template expansion内部に、SMIRKS左辺のquery原子・結合総数、明示的な元素個数、接続元素
ペア結合数の下限を使う事前絞り込みを追加した。prepared rule構築時に一度だけ署名を作り、
対象分子のtopology・元素・結合在庫が不足するtemplateは重い反応matching前に除外する。
alternative、negation、recursive SMARTS、wildcard、ring closure、未知構文はfail-openで
数えず、候補欠落を避ける。
固定3-target・depth=5・beam=100の2反復平均では、template expansionが3.240秒から
1.252秒へ61.4%短縮、全体が6.310秒から4.198秒へ33.5%短縮した。固定10-target・depth=2
では10/10 solved、raw候補787、dedup後547を維持し、expansionは37.8msから17.0msへ
短縮した。root lib 824テストとrelease buildを通過している。長いtemplate IDのlookupを
rule-address indexへ置き換える追加案は2反復平均で全体1.2%、展開3.0%悪化したため撤回した。

v1.0.3後の次期候補では、明示的な結合次数と芳香族／脂肪族atom classを同じfail-open
prefilterへ追加した。曖昧・暗黙・directional・複合query・ring closureは絞り込みに使わない。
さらに、安価なinventory判定をRayon reaction job投入前へ移し、対象結合inventoryを
inline SmallVec上のsort/run-lengthで構築、step costの一時Vecとdedup診断時のSMILES複製を
除去した。固定10-target・5,000-template・depth=2・beam=100のrelease smokeでは、変更前
3回平均725.63ms/targetから変更後5回平均663.82ms/targetへ8.52%短縮した。solved 1/10、
平均29 nodes、retro-cache hit/miss 18/272、raw候補21,661、cross-template dedup後14,700は
全runで一致した。500-template×10-targetのprepared/unprepared候補等価性gateもPASS。
これはtemplate expansion局所測定であり、AiZynthFinderとのmatched full-search速度比較には
外挿しない。次の競合比較は同一target・stock・timeout・depth/beam budgetで別途実施する。

同じ次期候補で、既定SA heuristicのring perceptionがmain-thread profileの約60%を占める
ことを特定した。nativeかつtimeout・custom value estimator・forbidden-element制約のない検索で、
未評価かつ非stockの独立precursorをまとめ、既存と同じSA scoreをRayonで先行計算して同じcacheへ
格納するよう変更した。timeout時のcancel cadence、custom estimatorのcall count、WASMの逐次実行は
変更しない。500-template×10-targetのprepared/unprepared等価性gateはPASS。
固定10-target・5,000-template・depth=2・beam=100では、直前5回平均663.82ms/targetから
SA並列化直後の5回平均349.96msへ47.28%短縮した。後刻のrelease再build後7回は背景負荷が
高く平均393.72ms、中央値381.31msだったが、直前実装比40.69%、最初の725.63ms baseline比
45.74%の短縮を維持した。solved 1/10、平均29 nodes、cache 18/272、raw 21,661、
diagnostic unique 14,700は全run一致した。matched AiZynthFinder full-search比較は未実施のため、
この局所結果だけで競合速度優位とは判定しない。後段の共有プロセス比較もcoverageと
throughputの証拠に限定し、isolated latency gateとは分離する。
追加のexact-H prepared prefilterは候補等価性gateを通過したが、最終5回平均が373.38msで
SA並列化単独の349.96msを安定して上回れなかったため撤回した。

> Historical throughput snapshot (superseded by the formal report above):
>
> v1.0.3後の次期候補では、対象ごとに185,042件のunion stockを再canonicalizeしていた
per-target harnessを、stock/templateを一度だけ読む共有プロセスarmへ切り替え、固定
VAL-200・5,000 templates・depth=5・beam=100・max-routes=1を完走した。RENKINはraw
129/200、stock-terminal 128/200、reaction validator-confirmed 121/200で、残る8件は全て
stock確認済みdepth=0、stock外は1件だった。複合strict stock-terminalは128/200 (64.0%)。
固定AiZynthFinder 4.4.1行の123/200 (61.5%)に対するpaired差は+2.5pp、95% bootstrap CI
-4.5〜+9.5pp、McNemar p=0.576で、点推定は上回るが統計的優位は未証明である。
最終共有runはwall 593.75秒、target内探索時間p50/p95 0.618/10.681秒で、200/200行、
malformed/crash/hash欠落0、manifest開始終了時の全入力hash一致を確認した。
共有プロセスarmはper-target RSSと完全に同一のprocess-isolation costを持たないため、
競合速度優位の正式根拠には使わない。

SA先行計算の因果は、同じ次期候補binaryを機能ON/OFFだけ切り替えた固定40-target
full-searchで再測定した。ONは合計105.77秒、OFFは328.07秒で67.8%短縮し、40/40で高速、
route hash・展開node・候補数は完全一致した。小規模smokeだけでなく深い探索でも有効と
確認した一方、この値をAiZynthFinder速度へ外挿しない。比較aggregateには共通validator
合格またはstock確認済みdepth=0を要求する複合指標を追加し、batch runnerはrowsに加えて
aggregateと開始／終了manifestを生成可能にした。
未成功行のtool-native route countを`0`ではなくschema規定どおり`null`にする修正も加え、
正式arm verifierのnullability・ID・route hash・manifest検査を全て通過した。

同じ固定40-targetで、childごとのdefault heuristicを「保持frontier prefix 1回＋追加
precursorの差分fold」へ変更し、105.77秒から99.86秒へ5.6%短縮した。続く実プロファイルで、
標準化済み生成precursorのstock missに対し、SMILES parse・standardize・canonicalizeを
再実行していることを特定した。外部rootだけをparsed moleculeから事前解決し、生成物は
canonical stock keyを直接照合する実装へ変更した結果、99.86秒から74.09秒へさらに25.8%
短縮し、40/40対象が高速化した。aggregateのp50は1.219秒から0.980秒、p95は8.564秒から
5.983秒へ改善した。成功24/40、全route hash、展開node、候補数は一致している。最初の
直列SA arm 328.07秒からの累積短縮は77.4%である。direct generator precursorも同じ
標準化・molecule intern経路へ接続し、設定時のcache欠落を除いた。

追加の割当削減は測定で選別した。`PathNode.target`の`Arc<str>`化は0.14%悪化、negative
stock missを保持しない案は3.39%悪化したため撤回した。改善後profileでは候補生成とexact
SA評価がともにRayon上でCPUを使い切っており、次の大幅改善にはbeam上位候補だけを厳密評価
する設計が必要と考えられたが、既定SAの`[1.0, 1.5]`上下界による安全な事前除外は固定40件で
74.09秒から87.61秒へ悪化した。初対象でも最終beam eviction 975件のうち事前に証明できたのは
28件だけで、stock走査と境界計算を回収できなかった。target間でSA scoreを共有する上限付きcacheも
77.68秒で改善を示さなかった。両案ともroute hash・nodes・候補数の同一性は通過したが実装から撤回し、
この局所最適化系列はいったん停止する。再開時はchematic側matching/canonicalization改善など、
支配コストそのものを下げる手段が必要である。これらの
共有process値はthroughput evidenceであり、AiZynthFinderへの正式latency勝利にはしない。

同時に、file-backed TemplatePolicy/RetroGenerator/ValueModelのloaderとnative依存importを
call-site単位でWASMから除外した。`wasm-pack build --target nodejs --no-default-features`と
`examples/quickstart.mjs`を通過し、モデル未設定のWASM経路を維持した。

> Historical recovery snapshot (superseded):
>
> VAL-200 recovery arm（union stock・schematic 1.0.8）は完了した。RENKINは
`route_to_configured_stock` **135/200 (67.5%)**で、baseline 121/200 (60.5%)から
14件改善し、AiZynthFinder 123/200 (61.5%)を12件・6.0pp上回った。paired configured-stock
内訳は両者成功101件、AiZynthFinderのみ22件、RENKINのみ34件、両者失敗43件である。
一方、strict validator-confirmedはRENKIN 127/200 (63.5%)、AiZynthFinder 134/200
(67.0%)で、RENKINが3.5pp下回る。RENKINのp95 wall-clockは57.95秒、p95 RSSは約76.2MiB
であり、AiZynthFinderのp95 13.10秒・約545MiBに対して、速度は遅く、メモリは小さい。
したがってG1はcoverage勝利だが、複合勝利ゲートは未達。次はstrict validationの差分を
回収段階別に分析し、invalid/partial routeを増やさずに改善する。

次の競合latency gateではper-target timeoutと独立RSSを揃え、AiZynthFinder／ASKCOSとの
現行master比較を再実行する。公開時は、shared-stock、native-stock、strict validation、
p50/p95、memory、インストール条件を分離し、「どの条件で、どの指標を、どれだけ
超えたか」を示す。

## References

- [AiZynthFinder](https://github.com/MolecularAI/aizynthfinder) — neural expansion policy、MCTS、stock/policy構成
- [ASKCOS v2 documentation](https://askcos-docs.mit.edu/) — template relevance、fast filter、MCTS、条件関連モジュール
- [Syntheseus](https://microsoft.github.io/syntheseus/stable/) — 反応モデルと探索アルゴリズムの共通化、Retro*／MCTS／PDVN、benchmark
- [Chemformer](https://github.com/MolecularAI/Chemformer) — SMILES seq2seq／事前学習による反応・逆合成モデル
- [RetroGFN](https://arxiv.org/abs/2406.18739) — 多様でfeasibleな候補生成を目指すGFlowNet系モデル
- [OpenRetro](https://github.com/coleygroup/openretro) — one-step retrosynthesisの再現可能なbenchmark基盤
- [SynPlanner](https://synplanner.readthedocs.io/en/latest/) — 反応データ整理、ルール抽出、policy/value network、MCTS、route clusteringの統合pipeline
- [RENKIN comparison guide](docs/guides/open-source-retrosynthesis-comparison.md)
- [RENKIN historical program audit](docs/roadmap/renkin-85-program.md)
