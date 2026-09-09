# Full recovery policy diagnostic — fixed gap28

測定日: 2026-09-09

## 結果

- 対象: 固定 gap28 / shared-stock / 9998 templates
- baseline: 5/28
- full recovery: 9/28
- 追加回収: +4件
- regression: 0件
- timeout: 0件
- crash: 0件
- strict validated route: 9/28
- sweep wall time: 907.75秒
- total elapsed p50/p95: 21.64秒 / 76.08秒
- peak RSS p50/p95: 359 MiB / 414 MiB

## 解釈

native baselineが成功した対象は先に確定し、generatorや後段retryで置き換えない非押し出し条件を検証済み。full recoveryは前回のpropagated generator（7/28）から2件改善したが、既存のgraph selector（9/28）と同率である。したがって候補段階投入の安全性は確認できた一方、AiZynthFinderを同一条件で上回ったとは結論できない。

これは固定gap28の診断測定であり、VAL-200全体やAiZynthFinderとの正式な同一条件比較ではない。既定探索を変更せず、opt-in recovery policyとして扱う。

## 証跡

- `renkin.jsonl`: 対象別の全28行
- `renkin_aggregate.json`: 集計値
- `non_displacing_verification.json`: baseline/final/regression検証
