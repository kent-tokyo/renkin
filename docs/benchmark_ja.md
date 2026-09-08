---
title: "RENKIN ベンチマーク概要"
description: "RENKINのベンチマーク結果、比較条件、再現性の境界を簡潔にまとめたページ。"
---

# ベンチマーク概要

このページは結果の要約です。詳細な実験ノートや全ハッシュ、対象ごとの
失敗分析は、専用の比較ガイドとリポジトリ内の成果物に分離しています。

## 現在の比較

最新のVAL-200 shared-stock比較では、native routeの発見数はRENKINと
AiZynthFinder 4.4.1がともに134/200（67.0%）でした。

共通validatorとshared-stock終端を適用した厳格な指標は次のとおりです。

| ツール | 成功数 |
|---|---:|
| RENKIN | 134/200（67.0%） |
| AiZynthFinder 4.4.1 | 123/200（61.5%） |

これは固定cohortでの結果であり、CASP全般に対する普遍的な優位性や、
実験収率、化学者レビューの代替を意味しません。

[正式比較ガイド](guides/open-source-retrosynthesis-comparison.md)に、
プロトコル・validator方針・主張の限界を記載しています。

## 過去のUSPTO-50k stress test

過去の4,907件runは、小規模なconfigured stockを使ったroute-to-stock
stress testです。canonicalなsingle-step USPTO-50k課題ではありません。
v0.15.5時点の78.0%、95.9%、OOD 81.8%というheadline値は、ルールと
validatorの修正後に無効化されています。現在の性能として使用しないでください。

修正版snapshotの内部診断値は次のとおりです。

| 指標 | 結果 |
|---|---:|
| Search-to-stock | 986/4,907（20.09%） |
| Atom-balance filtered | 756/4,907（15.41%） |
| Rule-validator confirmed | 43/4,907（0.88%） |

いずれも過去の内部診断値であり、化学的正確性を表す率ではありません。

## 「解決」の意味

特に明記しない限り、`solved`は完全なrouteが見つかり、そのleafが設定した
stock policyを満たしたという意味です。USPTOの正解reactantとの一致や、
実験室での合成成功を証明するものではありません。

## ローカル再測定

```bash
cargo run --release --bin renkin-bench -- \
  --input data/uspto50k_test.smi \
  --depth 5 --beam-width 100 \
  --templates data/templates_extracted_5000.smi
```

planner間の条件固定比較は、[正式比較ガイド](guides/open-source-retrosynthesis-comparison.md)
を参照してください。
