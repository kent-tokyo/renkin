# RENKIN Roadmap

更新日: **2026-09-19**

基準リリース: **v1.0.8**（`4337b01`）。計画作成時のmain: `a2add72`。

RENKINの目標は、単一のsolved rateだけを最大化することではない。同じtarget、stock、
探索予算、validation policyで比較したときに、route coverage、化学的妥当性、速度、
再現性、監査可能性を同時に改善し、実験計画へ持ち込めるCASP基盤を作る。

この文書は今後の優先順位と完了条件だけを扱う。実装済み変更の詳細は
[`CHANGELOG.md`](CHANGELOG.md)、作業単位は[`tasks/todo.md`](tasks/todo.md)、測定結果は
`data/comparison/`と[`docs/benchmark.md`](docs/benchmark.md)を参照する。

二つの目標を維持する。**探索性能の証明はPhase 55、次の製品改善はO8の経路診断・安全な実行境界**。
O7はv1.0.8で出荷済み。監査機能の完成をAiZynthFinderへの性能優位性と取り違えない。

- **Phase 55** — 候補被覆・候補保存・下流到達性・非置換recoveryを個別に検証し、採用構成を
  凍結した独立TESTで比較する。同一stock・予算でnative/common-strictがともに上回るまで
  優位性を主張しない。[ローカル詳細計画](docs/roadmap/aizynthfinder-accuracy.md)
- **O8 Trusted route operations** — O7の証跡を土台に、step/route診断、binding間の意味の一致、
  machine-readableな実行上限を整える。その後、情報損失を追跡した外部route取込と限定的な修正提案へ進む。

次候補の仮称は**v1.0.9候補**。必須範囲は下記のO8.0–O8.2の限定sliceとし、3か月分を一度に出荷しない。
この計画更新では版番号変更・公開・依存更新・新規benchmark実行は行わない。

## Status legend

| Status | 意味 |
|---|---|
| Shipped | 公開リリースに含まれ、回帰gateを通過済み |
| Implemented | working treeで実装・局所検証済み。release gateは未完了 |
| Partial | 一部の成果物は実装済みだが、当該phaseの完了条件は未達 |
| Active | 現在の優先開発対象 |
| HOLD | 実測で採用条件を満たさず、既定値へ昇格しない |
| Gated | 実装候補または継続領域。現在のP0を閉じ、必要な原因証拠が得られるまで新規実装を始めない |
| Blocked | 外部環境または必須artifactが揃うまで完了不能 |
| Planned | 前段のgate通過後に着手 |
| Continuous | 各リリースで継続する横断作業 |

## Current position

| 保存済み証拠 | 確認できること | 未達・主張の限界 |
|---|---|---|
| 旧VAL-200 | nativeは134/200対134/200。共通strictは134対123、95% CI [-1.5, +12.5]pp | 優位性未証明。新releaseの測定値ではない |
| 2026-09-16 r2独立TEST | 登録済みshared-stock構成でnative/strictともRENKIN 481/690、AiZ 32/690。差+65.07pp、95% CI [+61.45, +68.55]pp | **この構成のcoverage gateは通過**。RENKIN revisionは`b98a5c1`、v1.0.8の比較ではない。RSS・初回解時間は未計測 |
| O7 / v1.0.8 | canonical v1再import・receipt結合、v2明示tree API/export、工程指標・入力来歴・監査ranking・機構証跡 | 実procedure検証はO7.0–O7.1の一例。O7.2–O7.4の実利用検証は継続。v2のreceipt replayまで対応済みとはしない |

r2の速度差は**両者が解いた32件に限る**条件付き観測であり、全体の速度優位性ではない。
旧VALとr2のAiZ成功率の大差は、stock・cohort・model・設定の違いを説明する対象とする。
同一stockだけで全条件が公平とみなさず、計測・設定の誤りが確認された場合は訂正receiptを追加する。
結果が低いという理由だけで設定を変えたり、r2結果を消したりしない。

[r2正式結果](docs/benchmark/phase55-r2-result-20260916.md)、
[指標定義](docs/benchmark/phase55-metric-truth-table.md)、
[O7運用記録](docs/benchmark/o7-operational-validation-20260916.md)を証拠の正本とする。
r1はAiZ YAML hashの事前固定不足によるrehearsal、50件smokeは接続確認であり、正式比較に混ぜない。

## 競合変化とRENKINの境界（2026-09-19確認）

| 一次情報 | 判断 | RENKINの対応 |
|---|---|---|
| [AiZ PR #205](https://github.com/MolecularAI/aizynthfinder/pull/205): Python 3.13 / NumPy 2対応、open | 環境更新の提案。未出荷のroute変更とは扱わない | 4.4.1 fixtureを維持。新releaseを別compatibility laneで検証し、凍結比較のimageを更新しない |
| [SynPlanner v1.7.0](https://github.com/Laboratoire-de-Chemoinformatique/SynPlanner/releases/tag/v1.7.0): priority rules・reaction rebalancing等 | 経路品質と診断の拡張 | 現在のv1.6.0 fixtureから1.7.0実exportへの差分を調査。補完されたspeciesを観測事実と混同しない |
| [SynPlanner PR #114](https://github.com/Laboratoire-de-Chemoinformatique/SynPlanner/pull/114): 保護基修正・route mapping、open | route repairの設計シグナル。条件妥当性はPR自身も範囲外 | まず診断・producer/consumer整合。修正提案は後段、opt-in、元route保持 |
| [ASKCOSv2 core](https://gitlab.com/mlpds_mit/askcosv2/askcos2_core): MolScribe Web wrapper | 入力経路の拡張 | O7入力来歴をbrowser監査へ接続。OCRモデルは作らない |
| [RetroCast](https://github.com/batistagroup/retrocast): 共通route・adapter・評価 | schema変換単体は差別化しにくい | stock policy・情報損失・再監査で差別化し、必要な1形式だけinteropを検証 |

**chematic**は分子/反応のparse・identity・mapping・生成の低レイヤ、**RENKIN**はroute graph・
stock・探索・証跡・採否・修正提案を担当する。chematicのTrust holdout、RDKit.js比較、Python先行対応を
RENKINの作業へ移植しない。依存更新は必要API・MSRV・各bindingの回帰を確認して別commitで行う。

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

1. **55.0 / 計測の穴を閉じる** — r2の設定とartifactを監査し、peak RSS・time-to-first-routeの
   取得を実装する。未記録値は旧rowsから復元できないため`not_measured`を維持する。開発smoke後、
   新run ID・計測revision・両armの条件・解析法を事前登録した補足比較で測る。r2 coverageは置換しない。
2. **O8.0 → O8.1 / 経路診断の契約** — 既存validatorとbinding対応を棚卸しし、SynPlanner実exportの
   差分を採取。共通reason code・step/occurrence対応を追加し、不正・未対応・情報不足を分ける。
3. **O8.2 / 実行上限を外部へ示す** — 同じ定義からCLI/Python/WASM/MCPの対応範囲・上限・
   timeout/cancel方式を公開。browserの監査中断と安全な入力処理を先行sliceとして実装する。

探索改善は止めない。55.1で第一喪失点が観測できた場合だけ55.2/55.3/55.5の一仮説を単独A/Bへ送る。
測定中のコード・設定を凍結し、他の高負荷作業と重ねない。O8の出荷を性能比較の通過条件にはしない。

### O8 — 1〜3か月の段階計画

O8.0は実装済み、O8.1–O8.2は着手済みで、O8.3–O8.4は**Planned**。期間は着手からの目安であり納期保証ではない。
Phase 55の長時間実行は開発と計算資源を分離する。

| Phase / 優先度 | 目安・依存 | 成果物・対象 | 合格条件 |
|---|---|---|---|
| **O8.0 / P0 契約・fixture棚卸し** | Implemented | `validation/`・`bridge/`・各bindingの対応表。出所/版/hash/利用条件付きSynPlanner 1.7.0 fixtureと未対応field一覧 | v1.7.0 upstream route 58、release/tag/commit/blob/file/license hashを固定。公開18-routeのnode fieldsを棚卸しし、adapter/CLIで2-step parse・normalize・forward replayを回帰。model全般・別wrapper・未merge PRは対象外 |
| **O8.1 / P0 Typed route diagnostics** | Active | rule/step/nodeを指すreason code、mappingとproducer/consumer診断、validation receipt | findingのoccurrence pathとstep参照を追加し、Rust/CLI/Python/WASMの共通wire意味を回帰。positive/negative/unsupported fixture拡充と高度なmapping診断は未完。invalidをpassにしない |
| **O8.2 / P0 Capability / limit contract** | Active | 各toolのschema/version・最大bytes/atoms/nodes/records・timeout/cancel・network・stable/experimental・refusal一覧。WASM/MCPは既存discoveryへ加算 | WASM/Python/CLI capability payloadを追加。実ブラウザで100,000-route監査の取消、Worker再生成、次監査成功、古い応答の非反映、console logなしを確認済み。公開最大値/max+1をRust・実Node/WASM・packaged Pythonで回帰。MCPは標準`tools/list`を維持し、統一payloadは未完 |
| **O8.3 / P1 Interop + browser evidence** | 4–8週、O8.1/2後 | SynPlanner 1.7.0 import回帰、O7入力来歴のlocal upload→audit→exportデモ。需要確認後に版固定RetroCast一形式を追加 | source node/occurrence・conditions・mapping・stock/proposal provenanceのfield別loss report。無損失対応範囲だけround-trip一致。入力構造の無断修正・remote取得なし |
| **O8.4 / P1 Bounded repair proposals** | 6–12週、O8.1/3後 | 診断に根拠がある1–2保護基familyに限定したopt-in提案。元route、before/after hash、actions、残存risk、予算、追加stock要求を保持 | 全候補を再監査し、invalid/unsupportedは採用しない。既存成功取消0、予算超過なし。開発で固定したfamilyを未使用評価集合で評価。条件未検証ならexperimental維持 |

#### O8.1で固定する化学的境界

- 既存`Valid / Invalid / NotEvaluable`とBridgeのpass/fail/partialを破壊しない。
  unsupportedとmissing evidenceを理由として区別し、既存statusへのversion付き変換を定義する。
- atom-map重複・曖昧な対応、selected product、同じ中間体の複数occurrence、cycle/no-opを診断する。
  mapping番号は反応ごとに局所的であり、別stepの番号不一致だけでinvalidにしない。
  producer/consumerの明示edgeと構造を基に照合し、mapping欠落は勝手に補完しない。
- **target element accountingは片方向の要素数チェック**。完全なmass/charge balanceではない。
  byproduct・試薬・量論が欠ける場合に完全収支や反応成立を宣言しない。構造、stock、forward、
  conditions、実験evidenceは独立axisで保持する。未対応の配位構造等を有効扱いしない。
- chematicの不足機能は最小再現・期待型・fixture付きupstream要望に切り出す。
  RENKINに第二のSMILES parserや低レイヤ反応engineを作らず、未提供APIは明示的に未評価とする。

#### O8.2–O8.4で守る製品境界

WASM `capabilities()`はstock/rule数に加え、監査形式・policy・入力上限・network/cancellation境界を返す。
playgroundの探索とauditはWorker破棄・再生成方式でcancel/timeoutを提供するが、coreの協調的取消ではない。
Pythonはnative search/audit/forward用の別payloadを返す。CLI/Python/MCPまで同一の取消・入力surfaceを
持つとは表現しない。
ファイルhash・parse・構造検査はローカル。OCR自体を外部で行った場合、その事実を保持し
「画像が一度も外部へ送られていない」とは主張しない。

repairは入力routeの上書きでも自動採用でもない。未記載試薬/副生成物を算術的に補って
validに変えない。保護・伝播・脱保護、最終target、stereo、追加leafのstock/policyを検証し、
収率や条件適合性は別evidenceがなければ未評価とする。安全な候補がなければ提案なしで終了する。
元の探索経路とrepair後の経路の成功率・コストを別集計し、既定探索への昇格はPhase 55のA/B後だけ。

### 次候補 v1.0.9の必須範囲とgate

- O8.0の対応表・出所付きfixture、O8.1の既存検査に対するreason codeとstep参照を必須とする。
  高度なmapping推論、全reaction family対応、repairは含めない。
- O8.2は既存toolのcapability/limit公開とbrowser auditの取消/timeoutを必須sliceとする。
  O7全sidecarの全binding公開や新adapterを抱き合わせない。
- opt-in未使用時の探索/stock/route hashを固定開発fixtureで維持し、追加診断の時間・RSSをA/B測定。
  許容回帰幅は測定前に登録し、超過時は修正または機能をopt-inへ戻す。
- 候補commitでworkspace test/clippy、WASM/browser、Python/MCP、旧schema、docs例を検証。
  宣言した全bindingの共通fixture、上限+1、取消後再実行、redactionをrelease blockerにする。
- 55.0計測instrumentationと開発smokeは必須。正式補足比較が残っていても監査製品は出荷可能だが、
  Phase 55完了や速度優位は主張しない。O8.3/4は別候補とし、必要ならsemverを再判断する。

### 今後の独立TESTの分岐（新protocolの登録時に固定）

| 判定 | 次に行うこと | 行わないこと |
|---|---|---|
| preflight不成立 | 不成立理由を修正し、protocol revision・新しいcohort・両armの再登録を行う | 片方のarmだけを都合よく再実行しない |
| 両主指標で優位 | 完全なreport、再現手順、release candidate gateへ進む | 別条件の数値を上乗せして優位性を拡張しない |
| 優位性未証明 | formal failure atlasから最大の救済可能な損失を一つ選び、55.2/55.3/55.5の単独VAL A/Bへ戻る | TEST結果に合わせた閾値・stock・候補の調整をしない |
| correctness/resource gate不成立 | 当該境界の修正と最小回帰fixtureを優先する | solved rateだけで採用しない |

既知のr2対象で行う補足計測は独立したcoverage証明ではない。新候補の選別に利用した対象は
閲覧済みとして記録し、新たな汎化性能の証明には未使用cohortを別登録する。

### 出荷済み: O7の維持と残る運用検証

O7.0–O7.4はv1.0.8に含まれる。実procedureとsource artifactでO7.0–O7.1の
再import・再監査・metrics対応を確認済み。実装済み機能を再度作らない。

| 優先度 / Phase（実装順） | Status | 成果物 | 完了条件 |
|---|---|---|---|
| O7.0 Evidence binding | Shipped | v1再import・local receipt結合、明示tree v2 API/export | v2 receipt replayとは区別し、既存統合fixtureを維持 |
| O7.1 Process metrics | Shipped / Operationally validated | `route_metrics_v1`、質量ledger、source artifact/receipt hash | 不足データは`not_evaluable`。複数step実procedureを追加検証 |
| O7.2 Input artifact | Shipped / Partial operational validation | content hash、変換履歴、OCR/model、review receipt | 実OCR入力・stereo reviewとO8.3のbrowser接続 |
| O7.3 Audit ranking | Shipped / Partial operational validation | hard gate後のPareto/weighted profile、感度receipt | 実profileを追加検証。missing/非互換単位は拒否 |
| O7.4 Mechanistic evidence | Shipped / Partial operational validation | 外部計算receiptと比較可能なranking axis | 実計算artifactを検証。DFT実行はしない |

O7.2–O7.4は実装済みのopt-in機能として回帰を維持し、実利用fixtureは入手後に検証する。
O7.0–O7.1は公開特許の工程例で再import・再監査し、水・workup・廃棄物の不足を
`not_evaluable`として記録した。詳細は
[O7運用記録](docs/benchmark/o7-operational-validation-20260916.md)を参照。
MolScribe・DFT本体は実装しない。既存の`atom_economy`は記載された
precursorのMW比であり、全量論試薬を扱う理論atom economyや実工程PMIへ読み替えない。

### P0 — 成功率の証明: Phase 55

| Phase | Status | 現在の証拠 | 次の判定 |
|---|---|---|---|
| 55.0 測定契約 | Implemented / Partial | r2のimage/config/asset hash、8 CPU/6 GiB、ledgerは検証済み。RSSと初回解時間は欠落 | 未記録値は復元しない。process tree/container全体のRSS、起動/探索/初回解timer、timeoutの打切りを定義し、新runで両armを計測 |
| 55.1 Failure atlas | Implemented / Partial | VAL-200を両者成功・片側成功・両者失敗へ分類。未観測原因はunknownとして保持 | 次の仮説に必要な第一喪失点を観測する。深さ/beam到達だけで原因確定しない |
| 55.2 Ordering-only model | HOLD | TRAIN-only ONNX VAL-200はstrict 121→124（+3pp、95% CI −1.5〜+5.0pp、McNemar p=0.549）。timeout 0→2、p95 8.39→18.11秒、RSS p95 209→387 MiB。軽量512×128も10件でstrict 8→9・timeout 0だがp95 8.39→52.09秒、RSS p95 204→332 MiB | template-ID対応を保ったまま推論コストを下げ、timeout=0・strict非悪化を満たす候補だけ再評価 |
| 55.3 Downstream reachability | Implemented / HOLD | shared-cache selectorを実装したが、小規模A/Bで精度向上未確認 | 全VALで成功取り消し0、strict非悪化、runtime正常なら採用 |
| 55.4 Non-displacing recovery | Implemented / r2 evaluated | v2 VAL-200はstrict 129→134、回収5、native/strict回帰0、timeout/crash/attempt欠落0。31秒以内でfreezeし、r2へ投入済み | 新変更は別A/B。r2 coverage通過を他構成・新releaseへ自動継承しない |
| 55.5 Missing proposals | HOLD | 70 direct proposalsを安全に投入したがroute未回収 | valid完成routeを増やせるfamily/modelだけ採用 |
| 55.6 Independent TEST | Coverage gate passed / Partial | r1はAiZ YAML hashの開始前固定欠落により正式比較には使わない。露出identityを除外したr2（N=690）は、RENKIN/AiZ image digest、31秒/rank-1 YAML hash、asset hash、8 CPU/6 GiB、解析法を事前登録した。両armは690/690・単独検証済みで、preflightとpaired解析によりnative/strictのCI下限>0を確認した | coverage結論を保持し、Phase 55 exit gateのperformance receipt不足を補完する。r2の設定変更・片arm再測定は行わない |

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

0–7は継続的な機能領域、55.xは成功率改善、O7.xは出荷済み証跡基盤、O8.xは次の製品改善を表す。
既存Phase/O番号は改番しない。

| Phase | Status | Scope | 次のgate |
|---|---|---|---|
| 0 Benchmark contract | Shipped / Continuous | manifest、paired sampling、failure taxonomy、formal report | 新しいarmも同一schemaで再現できること |
| 1 Candidate coverage | Gated | template被覆、direct proposal、non-displacing recovery | zero-positive削減、regression=0 |
| 2 Learned models | HOLD / Gated | TemplatePolicy、RetroGenerator、ValueModel、ordering-only評価 | 実モデルのpaired top-k改善とresource非悪化 |
| 3 Search platform | Gated | A*、beam、profiles、trace、Pareto、必要時MCTS | coverage/latency Pareto改善 |
| 4 Chemical correctness | Active / Continuous | structure、element accounting、ring/stereo、forward replay | strict pass非悪化 |
| 5 Stock and constraints | Shipped / Active | private stock、vendor/price/lead time/hazard/region policy | freshness・provenance付きroute選択 |
| 6A Evidence contract | Shipped / Active | evidence sidecar、review rubric、provenance | substrate-level coverage拡大 |
| 6B Feasibility | Gated | deterministic route diagnostics、fast-filter相当 | reaction-family別precision/recall |
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

O6.1〜O6.5は出荷済み。export・envelope検査・trace自己整合性と、化学的再監査は区別する。
v1再import・外部証跡の結合はv1.0.8のO7.0で接続済み。

| Phase | Status | Delivered contract |
|---|---|---|
| O6.1 Audit receipt | Shipped | tool/API、version、argument/result hash、status、failure code |
| O6.2 Stock policy | Shipped | vendor、価格、納期、region、hazard、banlistのleaf判定 |
| O6.3 Adapter loss report | Shipped | preserved/normalized/inferred/dropped/unsupported |
| O6.4 Canonical interchange | Shipped | canonical exportとstrict envelope検査。v1再import・再監査は出荷済みO7.0で接続 |
| O6.5 Agent replay | Shipped | retro → condition → forward traceとreceipt自己整合性確認。実入力・実結果・最終監査との結合はO7.0 |

O6はroute solved rateを直接改善する機能ではない。agent出力は、RENKINの構造検証、stock
policy、forward replayを通るまで化学的妥当性や調達可能性の証拠として扱わない。

## Evidence-chain regression gate（O7以降で維持）

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

v1.0.8は公開済み。WASM budget、MCP numeric/
timeout境界、stock path保護、comparison manifest identityの回帰を維持する。O7以降も
入力サイズ・深さ・件数の上限、機密情報の非出力、receiptの差し替え拒否を検証する。
個別releaseの通過をSecurity track全体の完了とは扱わない。

## Phase 55 exit gate

Phase 55完了には、凍結した一つのRENKIN構成とAiZynthFinder 4.4.1を同じtarget、stock、
timeout、max routes、host条件、validation policyで実行し、次をすべて満たす必要がある。

- native `route_found`差とcommon-strict route-to-shared-stock差のpaired 95% CI下限がともに0より大きい。
- invalid、既存成功のregression、timeout、crashが事前登録上限を超えない。
- p50/p95/p99、time-to-first-route、peak RSSを、その値を実測したrunのraw rows/receiptから再生成できる。
  計測だけの補足runはr2と別IDにし、同じ凍結探索構成でも異なるrunの値を一行に合成しない。
- warm/cold、process全体/探索本体、両者成功群/全targetを区別し、未解決targetの初回解時間を
  timeout値や0秒として扱わない。出力rank-1をstop-after-firstと同義にしない。
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
- chematicにroute schema/stock/plannerを移植せず、RENKINに低レイヤchemistryを重複実装しない。
- 未mergeの競合PRを出荷済み性能と扱わず、rebalancingでinventしたspeciesを実験evidenceへ昇格しない。
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
