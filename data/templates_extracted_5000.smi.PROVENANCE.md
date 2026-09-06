# Provenance: `data/templates_extracted_5000.smi`

Issues #98 and #100. This is an optional, gitignored local corpus
(`data/templates_extracted*.smi` in `.gitignore`). It is never loaded by
default and is not shipped in the crate, Python, or npm packages. A caller
must opt in with `--templates data/templates_extracted_5000.smi`.

## Current corpus

The corpus was regenerated on 2026-09-06 from the pinned USPTO-50K training
revision and supersedes the earlier corpus whose exact source revision was
unknown:

```sh
python3 scripts/extract_templates.py \
  --dataset bisectgroup/USPTO_50K \
  --split train \
  --dataset-revision 08a575f0546b2be57242997fd45f684d6814d5a9 \
  --top 5000 \
  --reactant-radius 1 \
  --output data/templates_extracted_5000.smi \
  --manifest data/templates_extracted_5000.smi.manifest.json
```

- Source: `bisectgroup/USPTO_50K`, train split, revision
  `08a575f0546b2be57242997fd45f684d6814d5a9`
- Input reactions: 40,008
- Raw unique templates: 10,553
- CheMatic-compatible simplified templates: 9,974
- Selected templates: 5,000
- Extraction errors: 0
- Reactant radius: 1
- SHA-256:
  `7e0d105d736ffc264c1e286ff4fc7497ad913b61b4fa15555afc465dca05231e`
- Size: 525,850 bytes
- Line count: 5,004 total (4-line header + 5,000 template lines)
- Extraction environment: Python 3; `datasets 5.0.0`,
  `huggingface_hub 1.16.1`, `rdchiral 1.1.0`, `rdkit 2025.09.3`

The adjacent `data/templates_extracted_5000.smi.manifest.json` records the
source revision, extractor hash, corpus hash, extraction parameters, and row
accounting. Its current SHA-256 is
`dafae64a48702ac63bc0ea8e560e005d93ca6e7850f2da3d372c43c0e0b3b9c5`;
unlike the large generated corpus, the manifest is not ignored and is intended
to be committed alongside this note.

## Loader and application verification

The release binary was run against the installed local file:

```sh
target/release/renkin doctor templates \
  --templates data/templates_extracted_5000.smi \
  --output json
```

Result: PASS. All 5,000 raw lines load as logical `RetroRule`s and all 5,000
are concretely applicable by the current engine: 1,538 directly and 3,462 via
bounded hash-atom expansion. This resolves the old one-line loader rejection
from issue #98 without weakening the parser or application checks.

## Superseded local corpus

The previous bytes are retained locally as
`data/templates_extracted_5000.legacy-517f6a08.smi` and remain gitignored:

- SHA-256:
  `517f6a084921141b6080c3827c75e6c51ac148455218695dee6e9712e3731517`
- Size: 525,797 bytes
- Line count: 5,004 total
- Loader result: 4,999/5,000
- Exact source revision and package versions: unknown

That legacy file is preserved only to keep earlier local measurements
auditable. New measurements must use the current pinned corpus and record its
manifest hash.
