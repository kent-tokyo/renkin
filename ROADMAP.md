# RENKIN Roadmap

更新日: **2026-09-13**

基準リリース: **v1.0.6**

RENKINの目標は、単一のsolved rateだけを最大化することではない。同じtarget、stock、
探索予算、validation policyで比較したときに、route coverage、化学的妥当性、速度、
再現性、監査可能性を同時に改善し、実験計画へ持ち込めるCASP基盤を作る。

この文書は今後の優先順位と完了条件だけを扱う。実装済み変更の詳細は
[`CHANGELOG.md`](CHANGELOG.md)、作業単位は[`tasks/todo.md`](tasks/todo.md)、測定結果は
`data/comparison/`と[`docs/benchmark.md`](docs/benchmark.md)を参照する。

最優先の開発計画は[Phase 55: AiZynthFinder成功率超え](docs/roadmap/aizynthfinder-accuracy.md)。
現在は55.0–55.5の実装候補を評価し、55.6の独立TEST cohortを凍結済みである。精度改善は、
候補被覆、候補保存、下流到達性、非置換recoveryを別々に検証し、採用した構成だけを評価する。
同一stock・時間予算でnative成功率と共通検証後の成功率をともに上回るまで、優位性は主張しない。

## Status legend

| Status | 意味 |
|---|---|
| Shipped | 公開リリースに含まれ、回帰gateを通過済み |
| Implemented | working treeで実装・局所検証済み。release gateは未完了 |
| Active | 現在の優先開発対象 |
| HOLD | 実測で採用条件を満たさず、既定値へ昇格しない |
| Blocked | 外部環境または必須artifactが揃うまで完了不能 |
| Planned | 前段のgate通過後に着手 |
| Continuous | 各リリースで継続する横断作業 |

## Current position

保存済みVAL-200比較では、RENKINとAiZynthFinder 4.4.1のnative `route_found`はともに
134/200（67.0%）。共通構造検証・stock判定では134対123だが、paired 95% CIは
[-1.5pp, +12.5pp]、McNemar p=0.1608で優位性は未証明。統計集計は実施済みであり、
残るのは同一予算の再比較と独立TESTでの検証である。

別条件のgraph-selector recoveryは同一run内で133→143/200、成功取り消し0件だった。
9,974 templates・最大5 routes・120秒wrapperを使う候補段階の結果で、競合超えとは扱わない。
また再測定の`validator-confirmed` 126/200と共通strict 134/200は別指標であり、
forward確認済み134件とは表記しない。[根拠・指標定義](docs/roadmap/aizynthfinder-accuracy.md#evidence)

## Accuracy-first strategy

AiZynthFinderを超えるための主戦略は、単純なtemplate数・beam幅・深さの増量ではない。
VAL-200で確認された失敗を、次の4つの損失へ分けて順番に潰す。

| 損失 | 施策 | 成功の証拠 | 進めない条件 |
|---|---|---|---|
| 候補が存在しない | reaction-family単位のtemplate補完、検証済みdirect proposal | zero-positive群とvalid完成routeが減る | 特定targetのoracle注入、invalid増加 |
| 候補が早く消える | ordering-only、既存候補を保持するquota、候補集合hash | candidate recall不変、成功取り消し0 | 候補集合の不可逆削除、mapping不一致 |
| 候補はあるが終端へ届かない | bounded downstream reachability、段階recovery | same-budgetで未解決だけが回収される | p95/RSS悪化、既存成功の回帰 |
| 改善が開発集合だけに適合する | frozen independent TEST、paired解析 | native/common-strictのCI下限>0 | TESTを見ながら候補選択、条件変更 |

実装順はこの表の上から固定する。各施策は単独A/Bで採否を決め、採用した変更だけを
次段へ渡す。候補数の増加、参照routeとの一致、raw `route_found`だけでは精度改善と
みなさず、共通strict・invalid率・予算内完了を同時に確認する。

## North-star gates

比較runはtarget split、stock、template/model/data provenance、hardware、並列度、seed、
wall-clock/node budget、validation policy、configuration IDをmanifestへ記録する。

| 軸 | 主指標 | Release gate |
|---|---|---|
| Coverage | native route_found、strict route to shared stock | Phase 55の独立TESTで両指標のpaired 95% CI下限>0、効果量・regressionも報告 |
| Correctness | strict pass、atom loss、no-op、forward replay | coverage増加に伴うinvalid/partial増加なし |
| One-step | positive recall@k、invalid率、unique候補率 | zero-positive poolを減らし、候補集合を隠さない |
| Ranking | top-1、MRR、top-10 | ordering-only armで候補coverage不変を確認 |
| Performance | p50/p95/p99、time-to-first-route、peak RSS、timeout率 | 同一host・同一budgetで測定 |
| Trust | determinism、manifest replay、audit verdict | 完了・timeout・not-evaluable・chemical invalidを混同しない |
| Lab utility | condition/yield calibration、chemist preference | evidence、予測、abstentionを分離する |

## Priority order

### P0 — Phase 55: AiZynthFinderの成功率を超える

| Phase | Status | 現在の証拠 | 次の判定 |
|---|---|---|---|
| 55.0 測定契約 | Implemented / Blocked | metric truth table、manifest hardening、resume/output lock、fail-closed preflight | clean checkoutで両armの50-target smokeを通す |
| 55.1 Failure atlas | Implemented | VAL-200を両者成功・片側成功・両者失敗へ分類。未観測原因はunknownとして保持 | 採用候補を第一喪失点へ結び付ける |
| 55.2 Ordering-only model | HOLD | TRAIN-only ONNX VAL-200はstrict 121→124（+3pp、95% CI −1.5〜+5.0pp、McNemar p=0.549）。timeout 0→2、p95 8.39→18.11秒、RSS p95 209→387 MiB。軽量512×128も10件でstrict 8→9・timeout 0だがp95 8.39→52.09秒、RSS p95 204→332 MiB | template-ID対応を保ったまま推論コストを下げ、timeout=0・strict非悪化を満たす候補だけ再評価 |
| 55.3 Downstream reachability | Implemented / HOLD | shared-cache selectorを実装したが、小規模A/Bで精度向上未確認 | 全VALで成功取り消し0、strict非悪化、runtime正常なら採用 |
| 55.4 Non-displacing recovery | Implemented | 保存VALでbaseline 129→final 134、回収5、regression/timeout/crash 0 | 最終候補を同一総予算で再測定 |
| 55.5 Missing proposals | HOLD | 70 direct proposalsを安全に投入したがroute未回収 | valid完成routeを増やせるfamily/modelだけ採用 |
| 55.6 Independent TEST | Active / Blocked | provenance監査済み500-target cohortを凍結し、cohort preflightはeligible | Docker復旧後に両armを同一契約で完走・paired解析 |

55.0から55.5で採用条件を満たした構成を一つだけ凍結し、55.6へ送る。既存VALとgap cohortは
開発専用であり、独立TESTの代わりにしない。凍結cohort、hash、provenance、Docker blockerは
[独立TEST選定記録](docs/benchmark/phase55-independent-test-selection.md)へ集約する。

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

## Phase map

0–7は継続的な機能領域、55.xは今回の実行順を表す。既存Phase/O番号は改番しない。

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

O6.1〜O6.5は実装済み。今後は互換性維持と実利用fixtureの拡充を行う。

| Phase | Status | Delivered contract |
|---|---|---|
| O6.1 Audit receipt | Shipped | tool/API、version、argument/result hash、status、failure code |
| O6.2 Stock policy | Shipped | vendor、価格、納期、region、hazard、banlistのleaf判定 |
| O6.3 Adapter loss report | Shipped | preserved/normalized/inferred/dropped/unsupported |
| O6.4 Canonical interchange | Shipped | source IDとprovenanceを保持するroute import/export |
| O6.5 Agent replay | Shipped | retro → condition → forward traceとreceipt hash照合 |

O6はroute solved rateを直接改善する機能ではない。agent出力は、RENKINの構造検証、stock
policy、forward replayを通るまで化学的妥当性や調達可能性の証拠として扱わない。

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

直近のworking treeでは、WASM固有budget、厳格なelement filter、MCP checked numeric
conversion、標準検索timeout、stock path symlink拒否、comparison manifestのworktree/
configuration identityを実装している。release済みと混同せず、workspace test、WASM build、
MCP adversarial suite、release smokeを通過してからShippedへ移す。

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
- 「脆弱性ゼロ」や「普遍的優位性」を未測定のまま宣言しない。

## Evidence and references

- [Phase 55: AiZynthFinder成功率超えの実行計画](docs/roadmap/aizynthfinder-accuracy.md)
- [Benchmark methodology](docs/benchmark.md)
- [Open-source retrosynthesis comparison](docs/guides/open-source-retrosynthesis-comparison.md)
- [Historical 85-program audit](docs/roadmap/renkin-85-program.md)
- [AiZynthFinder](https://github.com/MolecularAI/aizynthfinder)
- [ASKCOS v2](https://askcos-docs.mit.edu/)
- [Syntheseus](https://microsoft.github.io/syntheseus/stable/)
- [OpenRetro](https://github.com/coleygroup/openretro)
- [SynPlanner](https://synplanner.readthedocs.io/en/latest/)
