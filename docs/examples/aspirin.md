# Aspirin Retrosynthesis

Aspirin (acetylsalicylic acid, ASA) is one of the world's most widely used pharmaceuticals. Its classical synthesis involves esterification of salicylic acid with acetic anhydride.

## Target

**Aspirin**: `CC(=O)Oc1ccccc1C(=O)O`

```python
import renkin

result = renkin.find_routes(
    smiles="CC(=O)Oc1ccccc1C(=O)O",
    depth=5,
    max_routes=5,
)

for route in result["routes"]:
    print(f"Route (depth {route['depth']}):")
    for step in route["steps"]:
        print(f"  {' + '.join(step['precursors'])}  [{step['rule']}]")
```

## Expected Routes

RENKIN finds two main disconnection strategies:

### Route 1: Ester cleavage (depth 1)

```
CC(=O)Oc1ccccc1C(=O)O
    → CC(=O)O + Oc1ccccc1C(=O)O
    [ester_cleavage]
```

- **Acetic acid** (`CC(=O)O`) — available from stock
- **Salicylic acid** (`Oc1ccccc1C(=O)O`) — available from stock

This corresponds to the reverse of the Fischer esterification / Einhorn procedure.

### Route 2: Acyl chloride route (depth 1)

```
CC(=O)Oc1ccccc1C(=O)O
    → CC(=O)Cl + Oc1ccccc1C(=O)O
    [aryl_ether_retro / friedel_crafts variant]
```

- **Acetyl chloride** (`CC(=O)Cl`) — available from stock
- **Salicylic acid** (`Oc1ccccc1C(=O)O`) — available from stock

This corresponds to the classical industrial synthesis using acetyl chloride.

## Try It

[**→ Open in Playground**](../playground/){ .md-button }

Enter `CC(=O)Oc1ccccc1C(=O)O` in the SMILES field to try interactively.
