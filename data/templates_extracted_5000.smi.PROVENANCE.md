# Provenance: `data/templates_extracted_5000.smi`

Issue #100. This file is optional, gitignored (`data/templates_extracted*.smi`
in `.gitignore`), and locally generated — never loaded by default, never
shipped in the pip/crate/npm packages. It's used only as a local diagnostic
when a caller explicitly passes `--templates data/templates_extracted_5000.smi`.

## Known facts (verifiable today, independent of generation history)

- SHA-256: `517f6a084921141b6080c3827c75e6c51ac148455218695dee6e9712e3731517`
- Size: 525,797 bytes
- Line count: 5,004 total (4-line header + 5,000 raw template lines)
- `load_rules_from_file` accepts 4,999 of the 5,000 raw lines as logical
  `RetroRule`s (1 rejected — a disconnected-fragment reactant SMARTS,
  unrelated to hash-atoms; see issue #98)
- 0 of the loaded 4,999 templates are `Unsupported` for concrete
  application (verified via the real binary, both before and after this
  round's issue #99 fix)
- The file's own header:
  ```
  # RENKIN extracted SMIRKS templates from USPTO-50k training set
  # Source: bisectgroup/USPTO_50K (train split, 40008 reactions)
  # Tool: rdchiral + simplification for chematic compatibility
  # Format: SMIRKS<TAB>count
  ```

## What can be reasonably inferred (not independently verified)

The header's `40008 reactions` figure matches exactly what
`scripts/extract_templates.py --dataset bisectgroup/USPTO_50K --split train`
would report as `len(ds)` for that dataset/split as of when this corpus was
generated (cross-confirmed: `scripts/generate_ring_context_metadata.py`'s
own module docstring independently describes "the full 40,008-reaction
USPTO-50K pass" against the same dataset). This makes it highly likely the
generating command was equivalent to:

```
python3 scripts/extract_templates.py --top 5000 \
    --dataset bisectgroup/USPTO_50K --split train \
    --output data/templates_extracted_5000.smi
```

(`--dataset`/`--split` match that script's own defaults, so they may not
have been passed explicitly.) The header's first line ("RENKIN extracted
SMIRKS templates from USPTO-50k training set") differs slightly from the
current script's hard-coded output ("RENKIN extracted SMIRKS templates",
no "from ... training set" suffix) — either an earlier script revision
wrote a different literal header string, or this file predates a header
wording change. Not investigated further; a cosmetic detail, not a content
concern.

## What is genuinely unknown (the real gap issue #100 identifies)

**The exact HuggingFace dataset revision.** Before this round,
`scripts/extract_templates.py` had no revision-pinning at all — its
`load_dataset(dataset_id, split=split)` call always resolved to whatever
HEAD happened to be at generation time, with no record kept. This file
predates that gap being noticed (per issue #100: "Produced in an earlier,
separate session, before the `[#N]` investigation began") and predates
this round's fix (see below), so:

- There is no way to determine which exact revision of
  `bisectgroup/USPTO_50K` produced this specific file.
- Even knowing the likely command above would **not** guarantee
  regenerating it today reproduces this file byte-for-byte — the dataset
  may have changed upstream since, and (per issue #72's own comment on a
  similarly-generated 500-template corpus) `rdchiral`'s extraction can
  legitimately tie-break equal-frequency templates in a different order
  across separate runs even with identical inputs.
- The exact `rdchiral`/`rdkit`/`datasets` package versions used are also
  unrecorded.

## What this round fixed (forward-looking, not retroactive)

`scripts/extract_templates.py` now pins a dataset revision by default
(`PINNED_DATASET_REVISION`, currently
`08a575f0546b2be57242997fd45f684d6814d5a9` — the same revision
`scripts/generate_ring_context_metadata.py` already pins for the same
dataset, for consistency), with `--dataset-revision <sha>` to override and
`--resolve-latest` to deliberately re-baseline against upstream drift —
mirroring `generate_ring_context_metadata.py`'s existing, already-reviewed
pattern exactly. The output file's `# Source:` header line now records
`{dataset_id}@{revision}` instead of just `{dataset_id}`, so any *future*
regeneration is self-documenting. **This does not retroactively recover
this file's own history** — only future regenerations benefit.

## Recommended next step (not done in this round — deliberately deferred)

If reproducibility of this specific 5,000-template corpus becomes
load-bearing for something (it currently isn't — issue #100 itself calls
this "not urgent," and nothing in the shipped product depends on it),
regenerate a fresh corpus with the now-pinned script and treat this file
as formally superseded, updating this note and every place that currently
cites this file's SHA-256 (e.g. cross-session working notes). Not done
here because: (a) not urgent per the issue's own framing, (b) overwriting
the current file would silently invalidate several already-recorded
findings tied to its exact current bytes this session (e.g. the
`extracted_9`/issue #72 template's exact position and content,
`load_rules_from_file`'s "4999 loaded" count from issue #98), and (c) a
full 40,008-reaction extraction pass is a genuinely heavy one-off
operation, not something to run incidentally as part of a documentation
fix.
