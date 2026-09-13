# Phase 55.0 metric truth table

更新: 2026-09-12

これはPhase 55の測定契約を固定するための記録である。`route_found`、validator、stock終端、
forward replayは別の指標であり、列の存在だけから同じ成功件数として扱わない。

## VAL-200 rerun row contract

対象: `data/comparison/formal_v1.0.3_candidate_20260909/formal_200_rerun_20260909/renkin.jsonl`

| 指標 | row条件 | 件数 |
|---|---|---:|
| completed | `run_status == completed` | 200/200 |
| native route_found | `route_found == true` | 134/200 |
| validator-confirmed | `validator_confirmed_route_found == true` | 126/200 |
| validator rejected | `validator_confirmed_route_found == false` | 0/200 |
| validator null | `validator_confirmed_route_found == null` | 74/200 |
| explicit not-evaluable | `not_evaluable == true` | 8/200 |
| configured-stock endpoint | `all_leaves_in_configured_stock == true` | 134/200 |
| combined strict | `route_found == true` かつ `validator_confirmed_route_found == true` かつ `all_leaves_in_configured_stock == true` | 126/200 |
| parseable among native | `route_found == true` かつ route tree/reaction steps がparseable | 134/200 |
| element accounted | `target_element_accounting_status == accounted` | 126/200 |
| timeout/crash | `run_status` が該当 | 0/200 |

validator nullの74件にはrouteなしの行も含まれるため、「not-evaluable route」を74件とは報告しない。
`all_leaves_in_configured_stock`だけでは構造検証を意味しない。旧reportの
「strict validated route 134/200」はstock endpointとcombined strictを混同した記載であり、
126/200へ訂正した。raw JSONLとaggregateは変更せず、訂正はreportとこの定義記録に残す。

## Phase 55 primary metrics

正式な主指標は、同じ固定cohort・stock・予算で全行を分母とする次の2つとする。

1. `native_route_found`: tool-nativeの正常完了結果。
2. `combined_strict_route`: parseable route、構造・element validator、configured stockを
   すべて満たすroute。

forward replayは`pass`、`fail`、`not_evaluable`を別列・別表で報告し、combined strictの代替にしない。
保存済みVAL-200は開発用・探索的な証拠であり、Phase 55.6の独立TESTでAiZynthFinderとの差を
paired bootstrap CIとexact McNemarで再評価する。
