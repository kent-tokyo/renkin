---
title: "O7 Operational Validation — Public Procedure Receipt"
description: "A reproducible local evidence-chain validation using a public ethyl-benzoate procedure, with missing process data retained as not evaluable."
---

# O7 operational validation: public procedure receipt

Date: 2026-09-16. This is an operational validation of the evidence chain,
not a reaction-performance, safety, sustainability, or competitor claim.

## Source and scope

The local source artifact is a concise, attributable transcription of numeric
fields from Example 2 of public patent publication
[CN115677497A](https://patents.google.com/patent/CN115677497A/en). It records
benzoyl chloride (940.2 kg), ethanol (318.2 kg), ethyl benzoate product
(956.8 kg), and the reported 60 °C / four-hour post-feed condition. It does
not redistribute patent text or make an assertion about the source's legal,
safety, or experimental validity.

The committed fixture is
`tests/fixtures/o7/ethyl_benzoate_cn115677497_procedure.json`. Its source
artifact hash in the receipt is
`sha256:eb2ff33dc1a57a13939fdaee44e43bcb09a114c5804a87353b034f1deecea8f8`.
The imported route is a curated one-step benzoyl-chloride + ethanol route; it
does **not** contain an atom-mapped reaction representation or claim a RENKIN
template. Consequently, its forward check is correctly `not_evaluable`.

## Reproduction

Build the candidate binary, then create a new local output directory:

```bash
cargo build --release --bin renkin
python3 scripts/validate_o7_operational_procedure.py \
  --binary target/release/renkin \
  --output-dir /private/tmp/renkin-o7-operational-validation-20260916
```

The script fails rather than overwriting an existing directory. It writes the
initial audit, canonical v1 interchange, local metrics sidecar, fresh-process
re-audit, and a manifest. The public report contains only source/sidecar/
receipt hashes, not the source URL or local source-artifact body.

## Recorded run

The candidate binary SHA-256 was
`sha256:ea5b733db1e2babcd5104bf0e3be694709113dab8df74fd7f9c8e74cdedef0ca`.
The route ID was
`7b22736368656d61223a2272656e6b696e2d697373756536362d726f7574652d686173682d7631222c2274726565223a5b2243284f43286331636363636331293d4f2943222c66616c73652c322c5b5b22432843294f222c747275652c302c5b5d5d2c5b224f3d4328633163636363633129436c222c747275652c302c5b5d5d5d5d7d`.

| Check | Result | Meaning |
|---|---|---|
| Canonical v1 re-import | Pass | Fresh-process re-audit reproduced the route ID and stock verdict. |
| Stock validation | Pass | The two stated precursors matched the supplied, explicit stock snapshot. |
| Forward validation | `not_evaluable` | No atom-mapped reaction representation was supplied. |
| PMI | `not_evaluable` | Water and workup quantities were required by the declared boundary but absent from the source artifact. |
| E-factor | `not_evaluable` | The same missing categories apply; total waste mass was also not reported. |
| Source redaction | Pass | The final re-audit JSON contains the source hash, never the source URL or artifact body. |

This is the intended outcome: reported starting-material and product masses
must not be promoted into a complete process metric by inferring density,
yield, utilities, water, workup, recycling, or waste. A future source with a
complete, compatible mass boundary may yield evaluated PMI/E-factor receipts;
this run does not.

## Candidate release gate

On candidate commit `e7ed5ce`, the following all passed: `cargo fmt --check`,
`cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D
warnings`, MCP integration tests, a locally built Python wheel/API smoke and
quickstart, wasm32 library build plus nodejs quickstart, live documentation
facts, and `mkdocs build --strict`. The final O7 receipt was regenerated from
the release binary in a fresh output directory. This gate does not publish a
release or establish Phase 55 performance superiority.
