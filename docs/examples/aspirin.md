# Aspirin Retrosynthesis

Aspirin (acetylsalicylic acid, ASA) is one of the world's most widely used pharmaceuticals. Its classical synthesis involves esterification of salicylic acid with acetic anhydride.

## Target

**Aspirin**: `CC(=O)Oc1ccccc1C(=O)O`

The output below is real, current output — reproduced with:

```bash
cargo run --bin renkin -- --target "CC(=O)Oc1ccccc1C(=O)O" --depth 5 --max-routes 3 --format tree
```

(equivalently in Python, see `examples/quickstart.py` — also run in CI, see
[Python API](../api/python.md) — which finds the same three routes via
`renkin.find_routes(target=..., depth=5, max_routes=3)`).

## Actual Routes Found

```
Target: CC(=O)Oc1ccccc1C(=O)O
Routes found: 3

Route 1  [score=1.09, depth=1]
OC(=O)c1ccccc1OC(=O)C
└── [co_aliphatic_cleavage]
    ├── O=CC  ✓ BB
    └── c1cccc(c1O)C(O)=O  ✓ BB

Route 2  [score=1.10, depth=1]
OC(=O)c1ccccc1OC(=O)C
└── [ester_cleavage]
    ├── OC(=O)C  ✓ BB
    └── c1cccc(c1O)C(O)=O  ✓ BB

Route 3  [score=1.10, depth=1]
OC(=O)c1ccccc1OC(=O)C
└── [aryl_ether_retro]
    ├── c1cccc(c1O)C(O)=O  ✓ BB
    └── OC(=O)C  ✓ BB
```

### Route 2: `ester_cleavage`

- **Acetic acid** (`OC(=O)C`) — available from stock
- **Salicylic acid** (`c1cccc(c1O)C(O)=O`) — available from stock

This corresponds to the reverse of the Fischer esterification / Einhorn procedure — the classical textbook disconnection for aspirin.

### Routes 1 & 3: `co_aliphatic_cleavage` / `aryl_ether_retro`

RENKIN's search also surfaces two additional graph-based disconnections
(`co_aliphatic_cleavage`, `aryl_ether_retro`) that reach the same
acetic-acid + salicylic-acid precursor pair through a different bond-breaking
path. These reflect generic C-O cleavage rules in RENKIN's rule set finding
multiple valid retrosynthetic justifications for the same physical bond — not
three independently useful synthetic strategies. `ester_cleavage` is the
chemically idiomatic one to cite; the other two are shown here for
transparency about what the search actually returns, not as recommended
routes.

## Try It

[**→ Open in Playground**](../playground/){ .md-button }

Enter `CC(=O)Oc1ccccc1C(=O)O` in the SMILES field to try interactively.
