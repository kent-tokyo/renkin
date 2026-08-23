# Syntheseus 0.8.0 fixture provenance

Companion to `tests/fixtures/syntheseus/0.7.2/PROVENANCE.md` -- these are
the same two fixtures (identical construction code, identical target
chemistry), regenerated against a real, artifact-pinned `syntheseus==0.8.0`
install, as part of the v0.31.0 Syntheseus 0.8 compatibility spike
(`docs/design/syntheseus-0.8-compatibility-spike.md`). Exists to prove
compatibility empirically -- not a hand-typed copy with the version string
edited.

## Software

- **syntheseus**: `0.8.0`, installed from the exact PyPI wheel artifact
  `syntheseus-0.8.0-py3-none-any.whl`
  (SHA-256 `c9bf6ea244badb209b7101a2d86b2b7ab40132b636e58bf09040dd2e7a66d32b`),
  downloaded via `pip download syntheseus==0.8.0 --no-deps` into a clean,
  isolated venv containing nothing else but this wheel and a locally-built
  `renkin` wheel (from this exact repo checkout, unmodified
  `python/renkin/syntheseus_exporter.py`). `importlib.metadata.version("syntheseus")`
  confirmed `"0.8.0"` at export time -- matches `pip show`, matches the
  installed artifact.
- **rdkit**: `2026.3.5` (pulled in as `syntheseus`'s own dependency) --
  identical to the version recorded in the `0.7.2` fixtures' own
  `PROVENANCE.md`, so this isn't a confounding variable between the two
  fixture sets.
- **Capture date**: 2026-08-22.

## Construction (byte-identical to `0.7.2`'s own fixtures)

Exact same code as `tests/fixtures/syntheseus/0.7.2/PROVENANCE.md`'s own
"Exporter script"/"Fixture A"/"Fixture B" sections -- not reproduced here
verbatim to avoid drift between two copies; see that file. The only
difference in this round's run: the installed `syntheseus` package is
`0.8.0`, not `0.7.2`.

## Compatibility result

Both fixtures are **semantically identical** to their `0.7.2` counterparts
in every field except `source_version` (`"0.8.0"` here, `"0.7.2"` there) --
confirmed by an automated diff, not eyeballed. `is_tree`/`is_minimal`/
`get_starting_molecules()` (Syntheseus's own reported properties) are also
unchanged: `is_tree: True`, `is_minimal: True`,
`get_starting_molecules() == {"CCO", "OC(=O)c1ccccc1"}` for Fixture A. See
`docs/design/syntheseus-0.8-compatibility-spike.md` for the full API-diff
report this round produced.

## Output SHA-256

- `linear_two_leaf_route.json`: `8781a084f2549fa695c422278612623196ed96d4bbd1e52adfb1963fb306c0ed`
- `convergent_route.json`: `1f1d50a642007a66ab451f5a435d94a4588715d2f4189545c579f59adb80b810`
