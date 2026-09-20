# SynPlanner v1.7.0 fixture provenance

`route_58_upstream.json` is an exact one-route slice of SynPlanner's committed
test data. It is not hand-authored and does not claim that RENKIN ran a
SynPlanner model.

- Release: `v1.7.0`, published 2026-08-25.
- Annotated tag object: `c581c67143a09c0dbffd3c5d019412ac54766e75`.
- Commit: `c46c84e7b3688a0175d87e7ba7e0302d139171a9`.
- Upstream file: `tests/data/routes_mol_1.json`.
- Upstream Git blob: `3f64c36f1746647b9c6678bd91a6e34aca681a27`.
- Upstream URL:
  `https://github.com/Laboratoire-de-Chemoinformatique/SynPlanner/blob/v1.7.0/tests/data/routes_mol_1.json`.
- Selection: top-level route ID `58`, the smallest serialized route in that
  upstream fixture. Selection was made for regression cost, not audit outcome.
- Extraction: `jq '{"58": .["58"]}' tests/data/routes_mol_1.json`.
- Extracted file SHA-256:
  `1de2e74148d479e0d27524ac3707d55b82caecbf64cc4e1d5664cd88a003bbe4`.
- Canonical route-value SHA-256 (sorted compact JSON, identical upstream and
  local): `f15f4adfec61e1c4c9b5ac8aa0cd8c258be19a474b77e5b358ab680375bba875`.
- The complete 18-route upstream file uses only three node-key sets:
  `type,smiles,children`, `type,smiles,in_stock`, and
  `type,smiles,children,in_stock`. No unsupported node field was observed.
- v1.7.0 JSON exporter blob:
  `96a564903b95b13c52c998b5106539e74ea320c0` at
  `synplan/chem/reaction/routes/io/json.py`.
- Capture date: 2026-09-20.
- License: MIT; the upstream text is preserved in `UPSTREAM-LICENSE`.

This fixture proves compatibility only with the public v1.7.0 route-tree JSON
shape and the chemistry represented by route 58. It does not prove
compatibility with every SynPlanner route, model checkpoint, protection-group
revision, or the separate CLI wrapper/bundle format.
