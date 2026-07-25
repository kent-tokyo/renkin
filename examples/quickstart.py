"""RENKIN Python quickstart. Runs as part of CI so this example can never
silently drift from the real API (see .github/workflows/ci.yml)."""

import json

import renkin

result = json.loads(
    renkin.find_routes(
        target="CC(=O)Oc1ccccc1C(=O)O",  # Aspirin
        depth=5,
        max_routes=3,
    )
)

print(f"Routes found: {result['routes_found']}")
for route in result["routes"]:
    print(f"Route (depth {route['depth']}):")
    for step in route["steps"]:
        print(f"  {step['target']} -> {' + '.join(step['precursors'])}")
        print(f"  via {step['rule']}")
