# RENKIN Roadmap

更新日: **2026-09-16**

基準リリース: **v1.0.7**（`ed41338`）

RENKINの目標は、単一のsolved rateだけを最大化することではない。同じtarget、stock、
探索予算、validation policyで比較したときに、route coverage、化学的妥当性、速度、
再現性、監査可能性を同時に改善し、実験計画へ持ち込めるCASP基盤を作る。

この文書は今後の優先順位と完了条件だけを扱う。実装済み変更の詳細は
[`CHANGELOG.md`](CHANGELOG.md)、作業単位は[`tasks/todo.md`](tasks/todo.md)、測定結果は
`data/comparison/`と[`docs/benchmark.md`](docs/benchmark.md)を参照する。

今後は二つの目標を分ける。**成功率の証明はPhase 55、次の製品候補はO7の運用検証・release gate**とする。
監査機能の完成をAiZynthFinderへの性能優位性と取り違えない。

- **Phase 55** — 候補被覆・候補保存・下流到達性・非置換recoveryを個別に検証し、採用構成を
  凍結した独立TESTで比較する。同一stock・予算でnative/common-strictがともに上回るまで
  優位性を主張しない。[ローカル詳細計画](docs/roadmap/aizynthfinder-accuracy.md)
- **O7 Evidence chain** — 入力・tool実行・route・監査結果を結び、根拠と不足データを示す。
  O7.0–O7.4の境界は実装済み。直近は実工程データによる検証と候補commitの配布前検証を行う。
  [ローカル詳細計画](docs/roadmap/evidence-chain.md)（公開対象外）

次候補の呼称は**v1.0.8候補**。この計画更新では版番号変更・公開・新規benchmark実行は行わない。

## Status legend

| Status | 意味 |
|---|---|
| Shipped | 公開リリースに含まれ、回帰gateを通過済み |
| Implemented | working treeで実装・局所検証済み。release gateは未完了 |
| Partial | 一部の成果物は実装済みだが、当該phaseの完了条件は未達 |
| Active | 現在の優先開発対象 |
| HOLD | 実測で採用条件を満たさず、既定値へ昇格しない |
| Blocked | 外部環境または必須artifactが揃うまで完了不能 |
| Planned | 前段のgate通過後に着手 |
| Continuous | 各リリースで継続する横断作業 |

## Current position

以下は保存済み測定であり、v1.0.7固有の性能値ではない。VAL-200比較では、RENKINと
AiZynthFinder 4.4.1のnative `route_found`はともに
134/200（67.0%）。共通構造検証・stock判定では134対123だが、paired 95% CIは
[-1.5pp, +12.5pp]、McNemar p=0.1608で優位性は未証明。統計集計は実施済みであり、
残るのは同一予算の再比較と独立TESTでの検証である。

別条件のgraph-selector recoveryは同一run内で133→143/200、成功取り消し0件だった。
9,974 templates・最大5 routes・120秒wrapperを使う候補段階の結果で、競合超えとは扱わない。
9月10日の別runではnative/stock endpointが134/200、validator-confirmedとcombined strictが
126/200だった。9月8日の共通strict 134件と混ぜない。
[指標定義](docs/benchmark/phase55-metric-truth-table.md)

9月15–16日の50件smokeは両arm完走、既存preflight通過。native/combined strictは
RENKIN 4/50、AiZynthFinder 1/50。ただし393件stock・500 templates・cold起動の接続検証であり、
正式計画の大規模共通stock・warm worker・top-5とは異なる。資源制限の実効性、実際の
AiZynthFinder設定・入力hashも未検証の部分があるため、**55.0全体は未完了**。
AiZynthFinder armのDocker接続はこのrunで確認したが、RENKIN armを同じ実効resource制限で
動かす比較containerは未準備である。詳細と時間集計訂正は
[smoke監査記録](docs/benchmark/phase55-smoke-review-20260916.md)を参照。

### 実行中の判断境界

9月16日に、55.4の既実装recoveryを開発用VAL-200で再検証した。これは、baselineと
同じcohort・stock・template・総予算で、既存成功を取り消さずに少なくとも一件を回収できるかを
判定する**候補選別**である。完走後にbaselineとのstrict回帰、全attempt、31秒budget、外側の
timeout/crashを検証するまでは、途中の行数やroute数を採用根拠にしない。

このrunはrank-1・native macOSの開発条件であり、正式比較の根拠ではない。結果を検証して
candidateをdevelopment-onlyで固定した後、別identityの未閲覧cohortと事前登録protocolを作成した。

独立TESTは `phase55-independent-test-20260916-001` として690 targetを凍結済みである。RENKIN armは
登録済みのrecovery v2構成（depth 5 / beam 100、recovery depth 6 / beam 200、外側31秒）で実行中であり、
実行中は探索設定、template、stock、image、評価scriptを変更しない。RENKIN完走後は同じcohort・stock・
資源上限でAiZynthFinder armを一回だけ実行し、両armのpreflightとpaired解析を通すまで結論を出さない。

Phase 55は次の二本の依存関係で進める。両方が閉じるまで独立TESTの**判定**は確定しない。

```text
開発候補: VAL-200 recovery v2 → 非悪化・予算検証 → development-only freeze
正式実行: container/resource/timing contract → development smoke → TEST protocol registration
                                                   両方の完了後 ↓
                                                             未使用TESTのpaired比較
```

前者は候補を選ぶ工程、後者は比較を信頼できるものにする工程である。candidate freezeは
独立TESTの結果ではなく、TESTの前に変更不能な開発判断を保存するためのreceiptとする。

## Accuracy-first strategy

AiZynthFinderを超えるための主戦略は、単純なtemplate数・beam幅・深さの増量ではない。
VAL-200で確認された失敗を、次の4つの損失へ分けて順番に潰す。

| 損失 | 施策 | 成功の証拠 | 進めない条件 |
|---|---|---|---|
| 候補が存在しない | reaction-family単位のtemplate補完、検証済みdirect proposal | zero-positive群が減り、valid完成routeが増える | 特定targetのoracle注入、invalid増加 |
| 候補が早く消える | ordering-only、既存候補を保持するquota、候補集合hash | candidate recall不変、成功取り消し0 | 候補集合の不可逆削除、mapping不一致 |
| 候補はあるが終端へ届かない | bounded downstream reachability、段階recovery | same-budgetで未解決だけが回収される | p95/RSS悪化、既存成功の回帰 |
| 改善が開発集合だけに適合する | frozen independent TEST、paired解析 | native/common-strictのCI下限>0 | TESTを見ながら候補選択、条件変更 |

この表は原因分類であり、全施策の再実装を要求しない。観測された損失と既存の負の結果から
次の仮説を選ぶ。各施策は単独A/Bで採否を決め、採用した変更だけを
次段へ渡す。候補数の増加、参照routeとの一致、raw `route_found`だけでは精度改善と
みなさず、共通strict・invalid率・予算内完了を同時に確認する。

## North-star gates

比較runはtarget split、stock、template/model/data provenance、hardware、並列度、seed、
wall-clock/node budget、validation policy、configuration IDをmanifestへ記録する。

| 軸 | 主指標 | 採用・主張のgate |
|---|---|---|
| Coverage | native route_found、strict route to shared stock | Phase 55の独立TESTで両指標のpaired 95% CI下限>0、効果量・regressionも報告 |
| Correctness | strict pass、atom loss、no-op、forward replay | coverage増加に伴うinvalid/partial増加なし |
| One-step | positive recall@k、invalid率、unique候補率 | zero-positive poolを減らし、候補集合を隠さない |
| Ranking | top-1、MRR、top-10 | ordering-only armで候補coverage不変を確認 |
| Performance | p50/p95/p99、time-to-first-route、peak RSS、timeout率 | 同一host・同一budgetで測定 |
| Trust | determinism、manifest replay、audit verdict | 完了・timeout・not-evaluable・chemical invalidを混同しない |
| Lab utility | condition/yield calibration、chemist preference | evidence、予測、abstentionを分離する |

上表の競合優位性gateと、監査機能の技術的release gateは別である。O7を出荷しても
Phase 55は完了扱いにしない。化学的にinvalidなrouteを指標の加点で合格に変えない。

## Priority order

### 直近の実行順

1. **55.6 / 凍結済み独立比較を完走** — 実行中のRENKIN armを同一commandで完走する。続いて登録済みのAiZynthFinder armを
   同じcohort・stock・資源上限で一回だけ実行する。途中結果で設定を変えない。
2. **55.0 + 55.6 / 契約・結果を検証して判定** — 両armのinput/image hash、CPU/RAM enforcement、timer receipt、
   output ledger、effective settingsをpreflightで検査し、native/common-strictのpaired CI・McNemar・資源を
   reportへ固定する。優位性の判定はこの時点だけで行う。
3. **55.0 / resume identityを補強** — 正式run完走後に、recovery depth・beam・timeoutをresume configuration identityへ
   含め、異なるrecovery予算での再開をfail-closedにする。進行中のrunには適用しない。
4. **55.1 → 55.5 / 次の候補仮説** — 55.6で優位性が未証明、または55.0がcandidateを実行不能と示した場合だけ、55.2/55.3/55.5を新しい原因証拠に基づく
   単独A/Bで再評価する。採用構成以外は正式比較へ持ち込まない。

製品側は並行してO7の運用検証を進める。性能測定中のコード変更・他の高負荷ジョブは避け、
新規OCR/DFT/MCTSやadapter増設より既存候補の検証を優先する。

### 製品候補: O7実装済み境界の運用検証

O7.0–O7.1をv1.0.8候補の必須範囲として維持する。実装済み機能を再度作らず、
実procedureとsource artifactで再import・再監査・metricsの対応を確認する。

| 優先度 / Phase（実装順） | Status | 成果物 | 完了条件 |
|---|---|---|---|
| P0 / O7.0 Evidence binding | Implemented | v1再importとlocal receipt sidecar結合、direct purchase/重複occurrenceを保持する明示tree v2 API・`--interchange-v2` export | body redactionを含む統合fixture、workspace・WASM lib・Python feature gateを通過 |
| P0 / O7.1 Process metrics | Implemented | `route_metrics_v1`、route hash結合ledger、source artifactを再hashするsidecar provenanceとreceipt hash | 手計算fixtureと一致。不足データは`not_evaluable`。実procedureは運用validationで追加 |
| P1 / O7.2 Input artifact | Implemented | image/SVG/PDF/textのcontent hash、変換履歴、OCR/model、正規化・review receipt | redacted reportとtarget bindingを実装。OCR/remote取得はlocal-first方針により別選択肢 |
| P1 / O7.3 Audit ranking | Implemented | hard gate後のParetoと固定normalization範囲のweighted profile、±10% sensitivity receipt | missing・非互換単位/境界は拒否。実profileは運用validationで追加 |
| P2 / O7.4 Mechanistic evidence | Implemented | 外部計算receiptと、同一step・quantity・unit・origin・computed contextのみを投影するranking axis | DFT実行なし。実計算artifactは運用validationで追加 |

O7.2–O7.4は実装済みのopt-in機能として回帰を維持し、実利用fixtureは入手後に検証する。
O7.0–O7.1は出典・利用条件の明確な工程例で手計算と照合し、不足項目を記録する。
実データ未入手時は運用検証を未完了とし、公開範囲・既知制限を候補判定へ残す。
MolScribe・DFT本体は実装しない。既存の`atom_economy`は記載された
precursorのMW比であり、全量論試薬を扱う理論atom economyや実工程PMIへ読み替えない。

### P0 — 成功率の証明: Phase 55

| Phase | Status | 現在の証拠 | 次の判定 |
|---|---|---|---|
| 55.0 測定契約 | Active | 小規模stockの50件smoke・既存preflight通過。RENKIN Linux image（`renkin-bench/renkin@sha256:b3b8…b2d`、OCI revision `e378e27`）を構築し、networkなし・8 CPU・6 GiB・read-only mountと`planner_timing_v1`をdevelopment smokeで確認。completed-invocation ledgerにより再開後sliceの総時間誤表示を防止。AiZ設定は追跡templateとbyte一致、参照HDF5/ONNX/template/filterをhash固定 | 両正式armの実効CPU/RAM上限・timer receipt・top-k意味・output ledgerをpreflightで検証し、結果reportへ結合。完走後にrecovery予算をresume identityへ追加 |
| 55.1 Failure atlas | Implemented / Partial | VAL-200を両者成功・片側成功・両者失敗へ分類。未観測原因はunknownとして保持 | 次の仮説に必要な第一喪失点を観測する。深さ/beam到達だけで原因確定しない |
| 55.2 Ordering-only model | HOLD | TRAIN-only ONNX VAL-200はstrict 121→124（+3pp、95% CI −1.5〜+5.0pp、McNemar p=0.549）。timeout 0→2、p95 8.39→18.11秒、RSS p95 209→387 MiB。軽量512×128も10件でstrict 8→9・timeout 0だがp95 8.39→52.09秒、RSS p95 204→332 MiB | template-ID対応を保ったまま推論コストを下げ、timeout=0・strict非悪化を満たす候補だけ再評価 |
| 55.3 Downstream reachability | Implemented / HOLD | shared-cache selectorを実装したが、小規模A/Bで精度向上未確認 | 全VALで成功取り消し0、strict非悪化、runtime正常なら採用 |
| 55.4 Non-displacing recovery | Implemented / development-selected | v2（同一VAL-200/stock/template、外側31秒、内部30秒）はstrict 129→134、回収5、native/strict回帰0、timeout/crash 0、attempt欠落0。recovery最大30,033.92ms、process最大30,408.53msで31秒以内。`phase55-recovery-v2-20260916`をdevelopment-only freeze済み | formal container契約下でこのconfigurationを固定して55.6へ渡す。55.0不成立または独立TEST未達なら結果を見て再調整せずHOLD |
| 55.5 Missing proposals | HOLD | 70 direct proposalsを安全に投入したがroute未回収 | valid完成routeを増やせるfamily/modelだけ採用 |
| 55.6 Independent TEST | Active | 閲覧済み50件・VAL 200件・smoke 10件を除外した690 targetを `phase55-independent-test-20260916-001` として凍結し、N=690・image digest/revision・31秒deadline・8 CPU/6 GiB・解析法を事前登録。RENKIN armを登録構成で実行中 | RENKIN完走後に同一protocolでAiZ armを実行し、preflight・paired解析・再現reportを通す。途中結果による再測定や設定変更はしない |

55.0から55.5で採用条件を満たした構成を一つだけ凍結し、55.6へ送る。既存VALとgap cohortは
開発専用であり、独立TESTの代わりにしない。凍結cohort、hash、provenance、閲覧履歴は
[独立TEST選定記録](docs/benchmark/phase55-independent-test-selection.md)へ集約する。
元の500件を変更せず、以後の独立性監査とprotocol改訂は別artifactへ記録する。新cohortが
必要なら既知結果の対象をID/構造で除外し、結果閲覧前にNを固定する。未観測の残り450件も
自動的に十分な独立TESTとは扱わない。O7の完成だけを理由に正式測定を開始しない。

### P1 — Competitive capability

1. **Model quality and generalization** — TRAIN-only学習、template-ID mapping、model hash、
   OOD/abstainを固定し、top-kだけでなくend-to-end coverageで採否を決める。
2. **Search portfolio** — A*/beamをbaselineとして維持し、`fast`、`balanced`、`deep`を
   versioned構成で比較する。MCTS/learned valueは必要性を測定してからopt-in追加する。
3. **SynPlanner parity lane** — data、rule extraction、policy/value、search、route clusteringを
   分離評価する。同じmodel、stock、budgetを共有できない比較は別laneとして表示する。

### P2 — Lab-facing expansion

1. **Evidence and feasibility** — substrate-specific evidence、conditions、stock provenanceを
   step単位で保持し、deterministic findingの上にhuman/LLM judgeを任意層として追加する。
2. **Conditions, yield, and selectivity** — evidence retrievalを先に出荷し、reported yieldと
   predicted yieldを分離する。temporal/OOD、calibration、abstentionを通るまでroute scoreへ
   統合しない。

### P2 — Upstream compatibility

AiZynthFinder 4.4.1、Syntheseus 0.7.2/0.8.0と既存SynPlanner adapterはgolden fixtureを維持する。
ASKCOSの画像入力はroute adapterから切り離す。ASKCOS/RetroCastの専用route adapterは
現行Bridgeにあると仮定せず、version固定schema・実例・ライセンスを確認してから別件で判断する。
releaseが同じことを「mainに変更がない」根拠にはしない。

## Phase map

0–7は継続的な機能領域、55.xは成功率改善の実行順、O7.xは監査製品の実行順を表す。
既存Phase/O番号は改番しない。

| Phase | Status | Scope | 次のgate |
|---|---|---|---|
| 0 Benchmark contract | Shipped / Continuous | manifest、paired sampling、failure taxonomy、formal report | 新しいarmも同一schemaで再現できること |
| 1 Candidate coverage | Active | template被覆、direct proposal、non-displacing recovery | zero-positive削減、regression=0 |
| 2 Learned models | Active | TemplatePolicy、RetroGenerator、ValueModel、ordering-only評価 | 実モデルのpaired top-k改善 |
| 3 Search platform | Active | A*、beam、profiles、trace、Pareto、必要時MCTS | coverage/latency Pareto改善 |
| 4 Chemical correctness | Active / Continuous | structure、element accounting、ring/stereo、forward replay | strict pass非悪化 |
| 5 Stock and constraints | Shipped / Active | private stock、vendor/price/lead time/hazard/region policy | freshness・provenance付きroute選択 |
| 6A Evidence contract | Shipped / Active | evidence sidecar、review rubric、provenance | substrate-level coverage拡大 |
| 6B Feasibility | Active | deterministic route diagnostics、fast-filter相当 | reaction-family別precision/recall |
| 6C–6D Recommendations | Planned | condition、yield、selectivity、feedback | held-out calibrationとabstention |
| 7 Product ecosystem | Continuous | CLI/Python/WASM/MCP、release、interop | cross-surface semantic consistency |

## Search profiles — O5

速度とcoverageを一つの設定へ押し込まず、用途別profileを明示する。

| Profile | 目的 | Policy |
|---|---|---|
| `fast` | 最初の候補を短時間で返す | bounded depth/beam、候補上限、早期終了 |
| `balanced` | 通常利用の速度とcoverage | fast相当から開始し、未解決だけbounded recovery |
| `deep` | 固定budget内でcoverage最大化 | 候補生成、policy、depth/beamを拡張 |

Status: profile schema、CLI/harness転送、configuration ID、10-target smokeは実装済み。
正式VAL-200でのcoverage、p95、RSS、strict validity比較は未完了。smokeの時間を競合比較や
正式性能値として扱わない。

O5 exit gate:

- 少なくとも二つのprofileが既存baselineを再現する。
- profile間のcoverage、latency、RSS、strict validityを同一manifestから再生成できる。
- timeoutや途中結果を成功として集計しない。

## Audit-native agent bridge — O6

O6.1〜O6.5の構成要素は実装済み。ただしexport・envelope検査・trace自己整合性の確認と、
外部証跡を再importして化学監査まで再実行することは異なる。後者は未公開のO7.0で接続済み。

| Phase | Status | Delivered contract |
|---|---|---|
| O6.1 Audit receipt | Shipped | tool/API、version、argument/result hash、status、failure code |
| O6.2 Stock policy | Shipped | vendor、価格、納期、region、hazard、banlistのleaf判定 |
| O6.3 Adapter loss report | Shipped | preserved/normalized/inferred/dropped/unsupported |
| O6.4 Canonical interchange | Shipped | 公開版はcanonical exportとstrict envelope検査。再import・再監査は未公開O7.0で接続済み |
| O6.5 Agent replay | Shipped | retro → condition → forward traceとreceipt自己整合性確認。実入力・実結果・最終監査との結合はO7.0 |

O6はroute solved rateを直接改善する機能ではない。agent出力は、RENKINの構造検証、stock
policy、forward replayを通るまで化学的妥当性や調達可能性の証拠として扱わない。

## O7 next-candidate exit gate

- schema/version、route/node identity、hash対象、情報損失、機密データのexport方針を先に固定する。
- ローカル入力 → audit → metrics receipt → export → 別processで再import・再監査を再現する。
- 欠落質量・非有限数・不一致hash・別routeのreceipt・不明な単位を正常値へ変換しない。
- PMI/E-factorは工程境界と実質量が揃った範囲だけ評価し、source報告値と再計算値を区別する。
- opt-in未使用時の既存CLI/MCP出力・探索結果・stock/structure判定をgolden fixtureで維持する。
- workspace test、clippy、WASM build、Python/MCPの公開surface回帰、docs例を候補commitで検証する。

証跡のhash一致は、実験成功・計算結果の正しさ・発行者の真正性を証明しない。署名や外部認証を
導入しない限りreceiptは自己整合性と入力結合を検査するものとして表示する。

## Security track

すべての外部入力を敵対的入力として扱う。`unsafe`を追加せず、入力拒否、resource停止、
chemical invalidを別のtermination reasonとして記録する。

| Track | Status | Remaining work |
|---|---|---|
| S0 Threat model | Shipped / Continuous | 新surface追加時に更新 |
| S1 Parser boundaries | Implemented | cross-surface differential fixture拡充 |
| S2 Resource limits | Active | template load・forward replay・auditのdeadline統一 |
| S3 File/provenance | Implemented | bundle/archive導入時の展開制限 |
| S4 MCP/API isolation | Implemented / Active | concurrency、request cancellation、ログ機密性 |
| S5 Supply chain | Continuous | advisory、license、workflow pin、artifact provenance |
| S6 Adversarial verification | Continuous | fuzz/property corpus、incident record、外部レビュー |

v1.0.7は候補commitで既存CI・配布gateを通過済み。以後もWASM budget、MCP numeric/
timeout境界、stock path保護、comparison manifest identityの回帰を維持する。O7ではさらに
入力サイズ・深さ・件数の上限、機密情報の非出力、receiptの差し替え拒否を検証する。
個別releaseの通過をSecurity track全体の完了とは扱わない。

## Phase 55 exit gate

Phase 55完了には、凍結した一つのRENKIN構成とAiZynthFinder 4.4.1を同じtarget、stock、
timeout、max routes、host条件、validation policyで実行し、次をすべて満たす必要がある。

- native `route_found`差とcommon-strict route-to-shared-stock差のpaired 95% CI下限がともに0より大きい。
- invalid、既存成功のregression、timeout、crashが事前登録上限を超えない。
- p50/p95/p99、time-to-first-route、peak RSSを同じraw rowsから再生成できる。
- target、stock、templates、model、binary、revision、budgetのhashと実効値をmanifestへ残す。
- 別のclean checkoutからpreflight、aggregate、統計、reportを再現できる。

一項目でも未達ならHOLD、実行環境が成立しないarmは`not_measured`とする。resume時は同じhashの
未完了targetだけを再開し、output ledgerの重複・並行writer・入力変更をfail-closedで拒否する。
段階別の対象コード、artifact hash、詳細な中止条件は
[Phase 55詳細計画](docs/roadmap/aizynthfinder-accuracy.md)に集約する。

## Explicit non-goals

- stock、model、hardware、timeoutが異なる数値を一つのleaderboardへ混ぜない。
- route発見を実験成功、収率、合成可能性の証明として表示しない。
- candidate欠落をrerankerだけで解決したと主張しない。
- validatorやstock identityを緩めてsolved rateを増やさない。
- ASKCOS、AiZynthFinder、Syntheseus、SynPlannerのUIや設定をそのまま複製しない。
- evidenceなしのcondition、yield、success probabilityを生成しない。
- SMILESや反応式だけからPMI/E-factorを捏造せず、既存MW比を工程サステナビリティと呼ばない。
- OCR confidenceを構造正解率と同一視せず、URLを渡しただけでremote取得しない。
- DFT実行・画像認識モデル・新しい検索戦略を、今回の監査receipt実装へ抱き合わせない。
- 「脆弱性ゼロ」や「普遍的優位性」を未測定のまま宣言しない。

## Evidence and references

- [Phase 55: AiZynthFinder成功率超えの実行計画](docs/roadmap/aizynthfinder-accuracy.md)
- [O7: Evidence chain詳細計画（ローカル・公開対象外）](docs/roadmap/evidence-chain.md)
- [Benchmark methodology](docs/benchmark.md)
- [Open-source retrosynthesis comparison](docs/guides/open-source-retrosynthesis-comparison.md)
- [Historical 85-program audit](docs/roadmap/renkin-85-program.md)
- [AiZynthFinder](https://github.com/MolecularAI/aizynthfinder)
- [ASKCOS v2](https://askcos-docs.mit.edu/)
- [Syntheseus](https://microsoft.github.io/syntheseus/stable/)
- [OpenRetro](https://github.com/coleygroup/openretro)
- [SynPlanner](https://synplanner.readthedocs.io/en/latest/)
