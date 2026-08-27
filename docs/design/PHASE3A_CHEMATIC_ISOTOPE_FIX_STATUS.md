# v0.36.0 Phase 2 — canonical-identity blockers, status

**Naming note**: this file's own history used "PR #196" as a placeholder
name for the eMolecules stock provenance retrofit, before that GitHub PR
number was actually assigned. It has since been consumed by an unrelated,
already-merged PR (the RENKIN-side chematic 0.16→0.20.1 dependency bump,
merged 2026-08-26 as `b7ab541`). Everywhere below, "the provenance
retrofit" refers to the still-not-yet-opened eMolecules work; "the
dependency-bump PR" refers to the real, already-merged PR #196.

Status: **All three of this investigation's own upstream fixes merged**
(Phase 3A/PR #389, Phase 3B/PR #392, Phase 3C/issue #390/PR #398 — CI all
green on each, all on chematic `main`), **and the resulting `chematic`
v0.20.1 release is published and consumed.** The user approved cutting a
v0.20.1 patch release; this session's own release PR (#401) was
superseded by the chematic repository owner's own equivalent PR (#410,
`release/v0.20.1-full`), who merged it and published `v0.20.1` to GitHub
Releases, crates.io, PyPI, and npm (`@kent-tokyo/chematic`) independently.
RENKIN's own Phase 4 (bump `chematic`/`chematic-rxn` from `0.16` to
`0.20.1`) is **done** — the dependency-bump PR (real #196, merged
`b7ab541`, 2026-08-26), full verification checklist passed (isotope
fixtures, 289-case tetrahedral corpus, the #390 E/Z fixture,
`data/building_blocks.smi` 402-compound stock doctor — now **PASS** where
it previously **FAIL**ed on `reimport_idempotency` — full RENKIN
workspace test suite, Python/WASM checks, package verification,
docs-facts checks, a small-scale retrosynthesis smoke test). Phase 5's
re-verification ladder is partway complete; see "What's still pending"
below for exactly where it stands.

Per the user's explicit 2026-08-25 grant ("今後しばらくの間あなた自身で自律的に
開発を続けてください。PRのmergeはあなた自身で行って構いません"), all PRs in
this investigation (both the three upstream chematic fixes and RENKIN's
own dependency-bump PR) were self-merged without a separate per-merge
approval round — version/release cuts remain excluded from that grant and
still require a stop-and-report (see the
`feedback_autonomous_merge_grant_2026-08-25` memory); the version cut
itself ended up being executed by the chematic repository owner, not this
session, so that carve-out was never actually exercised here.

## What Phase 3A/3B are

Two independent correctness fixes, in `kent-tokyo/chematic`, for defects
found while investigating a `renkin doctor stock reimport_idempotency`
FAIL during v0.36.0 Phase 2's eMolecules stock provenance retrofit
(itself still frozen, not yet opened as its own PR). See the
`diagnose/canonical-identity-blockers` branch's own commits for the full
local diagnostic tooling and Phase 0-2 evidence trail both fixes are
based on.

## Phase 3A — isotope stripping (MERGED)

- PR: **#389** — <https://github.com/kent-tokyo/chematic/pull/389> — **MERGED** (squash, branch deleted)
- Merge commit on chematic `main`: `cd7cf2f`
- `chematic-chem::hydrogen::remove_hydrogens` removed any atom with
  `element == H` unconditionally, silently destroying isotope labels
  (deuterium `[2H]`, tritium `[3H]`) on every canonicalization pass, not
  just re-canonicalization. Fixed: only a *non-isotopic* explicit H
  (`element == H && isotope.is_none()`) is removed; isotope-labeled H is
  kept as an explicit atom node. Heavy-atom isotopes (¹³C/¹⁴C/¹⁵N/¹⁸O)
  were never affected and remain untouched.
- Local verification before merge: `cargo test -p chematic-chem` 937
  passed; `cargo check --workspace` (excl. chematic-py/wasm/mcp) clean;
  `cargo test -p chematic-rxn -p chematic-smiles -p chematic --lib` 383
  passed; clippy/fmt clean.
- Real upstream CI: all 18 checks passed before merge (`Test` took 40m1s;
  `Rust Criterion regression gate` took 25m5s — this repo's CI is
  genuinely slow, not stuck, when checking back on future PRs here).

## Phase 3B — tetrahedral stereo non-idempotency (MERGED)

- PR: **#392** — <https://github.com/kent-tokyo/chematic/pull/392>
- Branch: `fix/restore-stereo-neighbor-order-in-remove-hydrogens`, based
  on `main` post-#389 merge (rebased once to resolve an expected conflict
  in `remove_hydrogens`/CHANGELOG.md — both fixes touch the same
  function; resolved by hand, re-verified after rebase).
- Root cause: `remove_hydrogens` never restored `Molecule`'s
  `stereo_neighbor_order`/`bond_directions` side tables (unlike its
  sibling `add_hydrogens`, which explicitly does). `chematic-smiles`'s
  canonical writer's `corrected_chirality` requires
  `stereo_neighbor_order` to safely reinterpret a stored `@`/`@@` tag
  against a reordered neighbor sequence; without it, the writer silently
  passed the raw tag through unchanged, which could flip a declared
  tetrahedral stereocenter to its mirror image on re-canonicalization.
- Fixed: `remove_hydrogens` now restores both side tables for every
  surviving atom/bond, the exact inverse of `add_hydrogens`'s own
  sentinel-remap.
- **Verified against the real 290-compound InChIKey-mismatch corpus from
  this investigation: 289 of 290 now match the true input identity** (up
  from 0 before this fix). The one residual case (a coupled/shared-bond
  E/Z oxime/hydrazone system) has a confirmed **different** root cause,
  independent of `remove_hydrogens` entirely — see below.
- Local verification: `cargo test -p chematic-chem` 947 passed;
  `cargo check --workspace` clean; `cargo test -p chematic-rxn -p
  chematic-smiles -p chematic --lib` 383 passed; `cargo test -p
  chematic-3d --lib` 594 passed excluding one test
  (`uff_only_rescue_now_preserves_stereo_for_atorvastatin_fragment`)
  confirmed to fail **identically** on a clean pre-fix `main` checkout
  (via a separate git worktree, byte-identical failure numbers) — a
  pre-existing, unrelated flaky/broken test, not a regression from this
  fix; clippy/fmt clean.
- Real upstream CI: all 19 checks passed before merge.
- Merge commit on chematic `main`: `743b77b`.

## Phase 3C — issue #390, coupled-bond E/Z anomaly (MERGED)

- Issue: **#390** — <https://github.com/kent-tokyo/chematic/issues/390>
  (auto-closed by the merge below, via the PR's own "Fixes #390").
- PR: **#398** — <https://github.com/kent-tokyo/chematic/pull/398> —
  **MERGED** (squash, branch deleted). Branch was
  `fix/ez-marker-frame-consistency`, based on `main` post-#392 merge
  (`743b77b`).
- Merge commit on chematic `main`: `0efed14` (2026-08-25T14:02:53Z).
- Real upstream CI: all 19 checks passed before merge (`Test` took
  40m13s).
- Witness: `O/N=C/C(C=N/O)=N\NC` (an oxime/hydrazone system: an ambiguous
  end with 2 candidate substituent bonds, one load-bearing for an
  unrelated double bond, one adjacent to a genuinely undefined third
  double bond). Confirmed independent of `remove_hydrogens`/`standardize`
  (reproduces with bare `parse`/`canonical_smiles`).
- Root cause, **two independent defects**, both required to reproduce and
  both fixed:
  1. `resolve_ez_markers`'s carrier election for the ambiguous end could
     elect the candidate whose sibling was raw-marked and load-bearing
     for a *different*, unrelated double bond — demoting the sibling
     silently under-specified that other double bond, while the elected
     candidate simultaneously handed a *third*, genuinely undefined
     double bond (InChI's own `?` descriptor confirms it) a geometry it
     never had. Fixed by `CanonicalWriter::is_load_bearing_elsewhere`
     (election-time protection; deliberately narrow — a sibling that is
     itself ambiguous, i.e. has its own resolution path, is not
     protected, so genuinely coupled/shared-carrier systems still
     resolve).
  2. Independently, `normalize_ez` seeded a shared E/Z group's sign from
     a value already re-oriented for one specific DFS write direction.
     Which end of a bond gets visited "forward" vs "backward" varies
     across candidate canonical numberings for reasons unrelated to that
     bond's own geometry (a tie elsewhere in the molecule), so the seeded
     sign varied too. Fixed by splitting `normalize_ez` into a
     mol-relative propagation step and a write-perspective
     anchor-seeding step (seeds once, never propagates in that frame).
  - Fixing defect 1 alone restored correctness for the witness but broke
    canonical-form stability (10 independently-rooted, RDKit-InChI-
    confirmed-equivalent respellings converged 10→1 before either fix
    existed, dropped to 3 non-idempotent strings with only defect 1
    fixed). An intermediate attempt at defect 2 (write-perspective-only
    seeding) restored stability but made canonicalization
    **informationally lossy**: a witness and its hand-verified E mirror
    both collapsed to the identical output. Caught by a
    mirror-distinctness test before combining both fixes into what
    shipped in PR #398.
- Verified: witness matches RDKit ground truth exactly; mirror (E/Z)
  distinctness holds; 18-way atom-order permutation invariance holds
  (same harness `EZ_SHARED_CARRIER_FULLY_RESOLVED`'s own regression test
  uses); fresh-process determinism (10 separate process invocations);
  full 290-record corpus verified **two** independent ways — idempotence
  **290/290** (up from 289/290) and independent RDKit InChIKey
  cross-check **290/290** (chematic's output reparsed in RDKit, compared
  against the InChIKey recorded for that record at investigation time).
  `cargo test -p chematic-smiles --lib` 206/206 (202 pre-existing + 4
  new); `cargo test -p chematic-chem --lib` 819/819; `cargo test -p
  chematic-rxn --lib` 180/180; clippy/fmt clean.
- A synthetic edge case found while writing this fix's own tests (not
  part of the filed issue, the 290 corpus, or any pre-existing test) — an
  ambiguous end whose both candidates carry mutually-consistent raw
  marks, where one candidate's sibling is itself adjacent to a genuinely
  undefined double bond, produced a mismatch between raw input and a
  canonicalize→reparse round-trip in this crate's own test-only
  `up_of_reference` oracle. **Isolated after opening the PR and ruled out
  as a production defect**: the raw over-specified input and the
  canonicalized+reparsed output were checked directly against RDKit
  (`MolToInchi`, per-bond `GetStereo()`) and encode the exact same real
  molecule/configuration — the mismatch is confined to the test-only
  oracle's own reference-substituent selection, not to
  `resolve_ez_markers`/`normalize_ez`. No fix needed, no issue filed.
  (This ruling-out is reflected in the PR's own description on GitHub and
  in chematic's own CHANGELOG.md wording, both already merged.)

## Follow-on requests filed against chematic (not fixes, not blocking)

Per the user's own "if there's something to request of chematic, file an
issue" invitation, two well-evidenced, narrowly-scoped requests came out
of this investigation (neither is a bug in itself, neither blocks Phase
4/5):

- **#393** — add a canonical round-trip idempotency property test
  (`canon(parse(x)) == canon(parse(canon(parse(x))))`) against chematic's
  own existing 5000-compound corpora (`scripts/descriptor_census_corpus.smi`,
  `scripts/chembl_accuracy_corpus_4999.smi`) — this exact test shape
  would have caught both #389 and #392 before they shipped, without
  needing another 9.47M-compound external scan. Already implemented on
  chematic `main`: `crates/chematic-smiles/tests/canonical_idempotency_corpus.rs` /
  `crates/chematic-chem/tests/canonical_idempotency_corpus_standardized.rs`.
  Running it found further real defects (independent of #389/#392/#390),
  tracked as chematic issues **#395**/**#399** — both fixed and shipped
  as part of the same `v0.20.1` release (PR #410, repository owner's own
  work, not this session's).
- **#394** — audit request: check whether other atom/bond-removing
  `MoleculeBuilder`-rebuild functions (a starting list of 8 candidate
  files noted in the issue, not individually confirmed broken) share the
  same silent `stereo_neighbor_order`/`bond_directions` loss pattern
  #392 fixed specifically for `remove_hydrogens`. Not independently
  followed up by this session; status on the chematic side unknown as of
  `v0.20.1`.

## What's still pending / not started

- **RENKIN's own Phase 4 is done.** `chematic`/`chematic-rxn` bumped
  `0.16` → `0.20.1` (dependency-bump PR, real GitHub #196, merged
  `b7ab541`, 2026-08-26) — see this repo's own `CHANGELOG.md` "Dependencies"
  entry under `[Unreleased]` for the full verification record.
- **Phase 5 (re-verification ladder), partial progress:**
  - Minimal fixtures — covered by Phase 4's own verification pass.
  - 290-case corpus — 0 mismatches (covered above, Phase 3C).
  - 12,684-row raw isotopic-H subset, extracted from the real
    `data/building_blocks_emolecules.smi` — **done**, 2026-08-26: **0
    parse failures, 0 lost H-isotopes, 12,684/12,684 preserved**,
    confirmed via `examples/isotope_sample_check.rs` (this repo) against
    chematic 0.20.1. A full 9.47M-row pass at this same per-row rate
    would take minutes, not hours, whenever authorized.
  - 402-compound `data/building_blocks.smi` stock doctor — covered by
    Phase 4's own verification pass (`renkin doctor stock
    reimport_idempotency` now **PASS**es).
  - 9.47M-row lightweight probe — **done**, 2026-08-26, as a *stratified
    sample* rather than an exhaustive pass (an exhaustive
    double-canonicalization pass over all 9.47M rows runs on the same
    order of wall-clock time as the full re-import it's meant to gate,
    defeating the point of a cheap intermediate rung). Sampled every
    190th data row of `data/building_blocks_emolecules.smi`
    (49,912 SMILES, evenly spread across the file) and ran each through
    up to 8 repeated `recanonicalize_stock_smiles` applications (the
    `diagnose/canonical-identity-blockers` branch's own
    `emolecules_idempotency_probe.rs`, run locally, not merged to
    master). Result: **0 parse failures; 0 cycles; 0
    `no_convergence_within_limit`** — every single line reaches a stable
    fixed point (49,911/49,912 after exactly 2 canonicalization calls, 1
    after 3), which is the actual signal this rung exists to check (that
    chematic's own canonical form, once produced, doesn't drift under
    repeated re-application). The naive "differs from the line stored in
    the file" count is a red herring here and should not be read as a
    defect count: 49,785/49,912 lines differ from pass 0, but pass 0 is
    text from the *old* ad-hoc Python/RDKit import script, not chematic's
    own canonical form -- of course chematic's canonicalization changes
    RDKit-canonicalized text, that comparison was never meaningful and
    will go away once the provenance retrofit regenerates this file via
    chematic itself. Separately, 29/49,912 lines are
    `structurally_unstable` (atom_count/formula/fragment_count differs
    between the stored line and chematic's canonical form) -- inspected
    by hand, every one is an organometallic bond-interpretation
    difference (Sn-C/Fe-C/Au-Cl treated as a real bond by the old RDKit
    export vs. split into separate fragments by chematic's valence model)
    or a bare `[H+]` counterion being dropped by `remove_explicit_h`
    (e.g. chloroauric acid's `[H+].[Au+3].4[Cl-]` losing its proton to
    become `[Au+3].4[Cl-]`, net charge -1). None of these 29 is an
    idempotency defect -- every one is still stable
    (`fixed_point_after_2_calls`), never a cycle or non-convergence. The
    organometallic bond-splitting is well-evidenced as a pre-existing
    tool-vs-tool representation difference; the bare-`[H+]`-counterion
    case is not fully verified the same way -- whether silently
    collapsing a salt and its conjugate base to the same stock identity
    is the *intended* `STANDARDIZE_OPTS` policy, or an unexamined side
    effect, is a separate open question this probe did not settle.
  - Single full re-import from raw eMolecules, and `renkin doctor stock`
    all-PASS on the result — **not started**, stays gated behind an
    explicit go-ahead given the disk footprint (~455MB+ output) and this
    project's standing heavy-measurement discipline; a general "continue
    developing autonomously" grant does not by itself authorize this
    step.
- **The eMolecules stock provenance retrofit itself** (a proper
  `StockManifest` — SHA-256, license, source revision — for
  `data/building_blocks_emolecules.smi`, replacing its current bare
  3-line comment header) remains not started, and has never been opened
  as its own GitHub PR. It resumes once the Phase 5 ladder above
  completes.
