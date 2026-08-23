"""RENKIN Python coverage-mode quickstart. Runs as part of CI so this
example can never silently drift from the real API (see
.github/workflows/ci.yml).

Coverage mode: Stage 1 (the default template set) runs first; only if it
finds nothing does Stage 2 run against a separately loaded, larger template
set. This target (N-phenylsuccinimide) is unsolvable by Stage 1's default
rules alone at this depth, but solvable once Stage 2 escalates to
tests/fixtures/coverage_mode_templates.smi's two extracted templates -- see
that file's header comment for how the target/template pairing was chosen.
"""

import json
import os

import renkin

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
COVERAGE_TEMPLATES = os.path.join(
    REPO_ROOT, "tests", "fixtures", "coverage_mode_templates.smi"
)

result = json.loads(
    renkin.find_routes(
        target="O=C1CCC(=O)N1c1ccccc1",  # N-phenylsuccinimide
        depth=2,
        max_routes=1,
        beam_width=100,
        search_mode="coverage",
        coverage_templates_path=COVERAGE_TEMPLATES,
    )
)

print(f"search_mode: {result['search_mode']}")
print(f"selected_stage: {result['selected_stage']}")
print(f"stage2_invoked: {result['stage2_invoked']}")
print(f"routes_found: {result['routes_found']}")
for route in result["routes"]:
    for step in route["steps"]:
        print(f"  {step['target']} -> {' + '.join(step['precursors'])}")
