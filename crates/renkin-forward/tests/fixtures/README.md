# Fixtures

`forward_bench_corpus.jsonl` is a small, **hand-authored, synthetic**
benchmark corpus for `renkin-forward benchmark` (see
`docs/guides/forward-benchmark.md`). It is NOT a real reaction corpus and
carries no accuracy claim about RENKIN's forward-prediction quality -- it
exists only to exercise every field and every `failure_reason` this harness
can emit against the embedded default rule set, deterministically, without
requiring a locally-downloaded external corpus (e.g. ORD) in CI.

Every `accepted_products` entry was derived empirically by running
`renkin-forward predict --report` against the given reactants and reading
off an actual candidate at the intended rank -- not hand-guessed chemistry --
so each row's expected `failure_reason` is a fact about the current engine
plus embedded rule set, not an assumption. See that guide's "Fixture corpus"
section for the full per-row rationale.
