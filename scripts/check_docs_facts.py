#!/usr/bin/env python3
"""Cross-checks docs/README against live facts derived from the current code
and data, so a stale count (like the 509->402, 31->28, 28->27 regression
this script exists to prevent) fails CI instead of silently shipping.

Usage: check_docs_facts.py <rule_count> <bb_file_count> <bb_fallback_count>
(all three come from `cargo run --example doc_facts`, computed live from
default_rules().len(), data/building_blocks.smi, and DEFAULT_BUILDING_BLOCKS
-- not hardcoded here. Two separate BB counts exist because CLI/Python only
use the 402-compound file when it's found relative to the cwd at runtime;
otherwise, and always for WASM, they fall back to the 152-compound compiled-in
set -- a real, user-visible distinction docs must not collapse into one.)

Pages like README.md/benchmark.md legitimately discuss old, invalidated
figures (78.0% pre-fix, 509 BBs before a later dedup, etc.) side by side with
current ones, behind a single prominent disclaimer near the top of the page --
not a per-line caveat. So a stale-looking figure is only an error when the
*whole document* has no disclaimer marker anywhere; that still catches a
regression landing in an undisclaimed page (e.g. an API doc), while not
flaring on every already-labeled historical row in README.md.
"""

import re
import sys
from pathlib import Path

STALE_FIGURES = [
    (re.compile(r"\b31\s*(hand[- ]?crafted|handcrafted|built-in)\s*rules?", re.IGNORECASE), "31 hand-crafted rules"),
    (re.compile(r"\b28\s*(hand[- ]?crafted|handcrafted|built-in)\s*rules?", re.IGNORECASE), "28 hand-crafted rules"),
    (re.compile(r"\b27\s*(hand[- ]?crafted|handcrafted|built-in)\s*rules?", re.IGNORECASE), "27 hand-crafted rules"),
    (re.compile(r"\b26\s*(hand[- ]?crafted|handcrafted|built-in)\s*rules?", re.IGNORECASE), "26 hand-crafted rules"),
    (re.compile(r"\b24\s*(hand[- ]?crafted|handcrafted|built-in)\s*rules?", re.IGNORECASE), "24 hand-crafted rules"),
    (re.compile(r"\b509\s*(building block|BB)", re.IGNORECASE), "509 building blocks"),
    (re.compile(r"78\.0%"), "78.0%"),
    (re.compile(r"95\.9%"), "95.9%"),
    (re.compile(r"81\.8%"), "81.8%"),
]

DISCLAIMER_MARKER = re.compile(
    r"invalid|invalidat|pre-fix|修正前|無効化|not.{0,20}re-measured|未再計測|historical|過去の|corrected baseline",
    re.IGNORECASE,
)

# Unconditional bans -- no legitimate historical use of these exists anywhere.
BANNED_PATTERNS = [
    (
        re.compile(r"\brenkin\.version\(\)"),
        "renkin.version() (Python API has no such function; use renkin.__version__)",
    ),
]


def main() -> None:
    rule_count = sys.argv[1]
    bb_file_count = sys.argv[2]
    bb_fallback_count = sys.argv[3]

    targets = list(Path("docs").rglob("*.md")) + [
        Path("README.md"),
        Path("README_ja.md"),
        Path("README_zh.md"),
    ]

    errors = []
    for path in targets:
        if not path.exists():
            continue
        text = path.read_text(encoding="utf-8")

        for pattern, message in BANNED_PATTERNS:
            if pattern.search(text):
                errors.append(f"{path}: contains banned pattern -- {message}")

        has_disclaimer = bool(DISCLAIMER_MARKER.search(text))
        if has_disclaimer:
            continue  # whole-page disclaimer covers every stale figure below it

        for pattern, label in STALE_FIGURES:
            m = pattern.search(text)
            if m:
                line_no = text.count("\n", 0, m.start()) + 1
                errors.append(
                    f"{path}:{line_no}: stale/invalidated figure ({label}) with "
                    f"no disclaimer anywhere in the document"
                )

    # Sanity check: the CURRENT correct facts should still be mentioned on the
    # primary landing pages -- catches the numbers being silently deleted
    # entirely, not just replaced with something else wrong. Both BB counts
    # must appear (not just the 402 file count) so a doc can't describe the
    # file-backed stock without ever disclosing the compiled-in fallback.
    for path in [Path("README.md"), Path("docs/index.md")]:
        text = path.read_text(encoding="utf-8")
        if rule_count not in text:
            errors.append(f"{path}: does not mention the current hand-crafted rule count ({rule_count})")
        if bb_file_count not in text:
            errors.append(f"{path}: does not mention the current building-block file count ({bb_file_count})")
        if bb_fallback_count not in text:
            errors.append(f"{path}: does not mention the current building-block fallback count ({bb_fallback_count})")

    if errors:
        print(f"Found {len(errors)} doc-facts issue(s):", file=sys.stderr)
        for e in errors:
            print(f"  - {e}", file=sys.stderr)
        sys.exit(1)

    print(
        f"OK: no stale facts found (rule_count={rule_count}, "
        f"bb_file_count={bb_file_count}, bb_fallback_count={bb_fallback_count})"
    )


if __name__ == "__main__":
    main()
