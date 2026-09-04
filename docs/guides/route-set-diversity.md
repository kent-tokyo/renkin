# Route-set chemical-idea diversity

RENKIN exposes `diversity::template_disconnection_cds()` as a deterministic
diagnostic for a set of `search::Route` values. It is motivated by the Chemical
Diversity Score described by Mrugalla et al., but it is intentionally a
**template-ID proxy**, not the paper's atom-mapped formed-bond CDS.

## Contract

1. Each route becomes a set of its distinct stable `template_id` values.
2. Duplicate sets are represented once.
3. A strict superset of another set is treated as a route variation and omitted
   from the core set.
4. The report applies all-to-all Jaccard-distance normalization to the remaining
   core sets. Zero or one core set returns `1.0`.

The result is deterministic and invariant to route ordering, duplicate routes,
and strict-superset variations. It measures diversity of recorded template
families under RENKIN's current route representation. Two templates can encode
the same formed bond, and one template can apply at different target bonds, so
the value must not be labelled or benchmarked as exact atom-mapped CDS.

## Provenance and licence boundary

- Research motivation and the formed-bond CDS definition: F. Mrugalla et al.,
  “Generating diversity and securing completeness in algorithmic
  retrosynthesis,” *Journal of Cheminformatics* 17, 72 (2025),
  <https://doi.org/10.1186/s13321-025-00981-x>, CC BY 4.0.
- RENKIN's implementation was written independently in Rust. No paper text,
  figure, dataset, trained model, or upstream implementation source is included.
- This diagnostic does not alter route search, candidate ranking, or stock
  membership.

This is a technical provenance record, not a freedom-to-operate opinion. Before
a commercial deployment that depends materially on this metric, obtain a
jurisdiction- and claim-specific patent review.
