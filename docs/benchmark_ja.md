---
title: "RENKIN ベンチマーク概要"
description: "RENKINのベンチマーク結果、比較条件、再現性の境界を簡潔にまとめたページ。"
---

# ベンチマーク概要

このページは結果の要約です。固定した条件での探索結果と、実験的な合成可能性・
速度の主張を区別します。入力hashや対象ごとの記録は結果文書に残しています。

## 登録済みの比較：Phase 55 r2

2026年9月16日の独立TESTは、固定した690標的、共通stock、asset、予算で
RENKIN `1.0.7`とAiZynthFinder `4.4.1`を比較しました。

| 指標 | RENKIN | AiZynthFinder | 対応のある差 |
| --- | ---: | ---: | ---: |
| 共通stockまでの厳格route | 481/690（69.71%） | 32/690（4.64%） | +65.07ポイント（95%信頼区間 +61.45〜+68.55） |

これは上記の構成でのcoverage結果です。現行v1.0.14の再測定、他のstock・
モデルでの優位性、実験成功、全標的の速度優位性は示しません。peak RSSと
最初のrouteまでの時間はこの比較では未測定です。

[Phase 55 r2の結果と測定条件](benchmark/phase55-r2-result-20260916.md)を確認して
から引用してください。

## 過去のVAL-200

保存済みのnative route発見数は両ツールとも134/200でした。共通validatorと
shared-stock終端を適用した厳格指標はRENKIN 134/200、AiZynthFinder
123/200です。これは開発用cohortの別構成であり、Phase 55と合算しません。
[比較ガイド](guides/open-source-retrosynthesis-comparison.md)に条件と限界を
記載しています。

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
