# RENKIN Workboard

更新日: **2026-09-23**
基準リリース: **v1.0.9**

この台帳は未完了の作業と、今後の判断に必要な完了済み証拠だけを置く。過去の
phase別進捗、micro-release、失敗分析はGit履歴、[CHANGELOG.md](../CHANGELOG.md)、
`data/comparison/`、`docs/benchmark/`に保存されている。ここへ完了済みのログを
再掲しない。

## Current objective

探索性能の追加主張には、coverageと速度を同じものとして扱わないmeasurement
receiptが必要である。同時に、O8で公開した診断・実行境界を、情報損失の見える
interopへ広げる。

## Next work

- [ ] **55.0 / performance receipt contract**
  - Phase 55 r2を置換せず、別run用にtool revision/model、cohort、stock、template、
    CPU/RAM/network、cold/warm、timeout、first-route event、process-tree RSSを固定する。
  - 両armのraw rows、manifest、row-completeness検証を保存する。RSSまたはfirst-routeを
    取得できないなら`not_measured`とし、速度優位性を主張しない。

- [ ] **O8.3 / loss-accounting interop**
  - SynPlanner 1.7.0 fixtureを起点に、source node/occurrence、conditions、mapping、
    stock/proposal provenanceの`preserved`/`normalized`/`inferred`/`dropped`/`unsupported`を
    field単位でreportする。
  - local input artifactをhash付きでauditし、browserからexportできる導線を追加する。
    remote fetch、OCR再実装、入力構造の無断修正は含めない。

- [ ] **55.x / first-loss diagnosis**
  - 追加探索を行う前に、候補欠落、ranking crowd-out、stock終端、validator rejectionの
    どこで失うかを固定fixtureで分類する。
  - 仮説は一度に一つだけA/Bし、同一cohort・stock・budgetでinvalidと既存成功取消が
    ないことを確認する。

- [ ] **O8.4 / bounded repair proposal**
  - O8.3の診断根拠がある1〜2 protecting-group familyに限る。
  - opt-in、元route保持、before/after hash、追加stock、残存risk、固定budget、全候補の
    再監査を必須にする。

## Completed baseline

- [x] **O8.0** — SynPlanner 1.7.0のprovenance固定fixtureとbinding対応表。
- [x] **O8.1** — `occurrence_path`/`step_index`、atom-map、producer/consumerの
  non-authoritative receipt。既存audit statusは変えない。
- [x] **O8.2** — CLI/Python/WASM/MCPのcapability/limit contract、browser auditの
  Worker terminate/respawn取消、上限+1拒否回帰。
- [x] **Phase 55 r2 coverage** — 690 targetのregistered shared-stock TEST。RENKIN
  481/690、AiZynthFinder 32/690は固定構成のcoverage evidenceであり、速度・実験成功・
  普遍的優位性ではない。

## Maintenance rules

- 新しいrelease候補は、feature単位のcommit、docs/CHANGELOG同期、affected bindingの回帰、
  package検証を揃える。
- upstream adapter更新はrelease/tag、source artifact、license、fixture hashを固定する。
- `tasks/lessons.md`は恒久的な実装上の教訓、`docs/benchmark/`と`data/comparison/`は
  measurement evidenceであり、このworkboardから削除対象にしない。

詳細な優先順位と完了条件は[ROADMAP.md](../ROADMAP.md)を参照する。
