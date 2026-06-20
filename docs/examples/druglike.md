# Drug-like Molecules

Examples of retrosynthesis on more complex, drug-like molecules.

## 4-Phenylpyridine (Suzuki coupling product)

SMILES: `c1ccc(-c2ccncc2)cc1`

This biaryl compound is typically synthesized via Suzuki-Miyaura cross-coupling.

RENKIN's graph-based `suzuki_retro` rule disconnects the biaryl bond:

```
c1ccc(-c2ccncc2)cc1
    → Brc1ccccc1 + OB(O)c1ccncc1
    [suzuki_retro]
```

or

```
c1ccc(-c2ccncc2)cc1
    → Brc1ccncc1 + OB(O)c1ccccc1
    [suzuki_retro]
```

Both bromobenzene (`Brc1ccccc1`) and 4-pyridineboronic acid (`OB(O)c1ccncc1`) are in the default building block stock.

## N-Phenyl-2-aminopyridine (Buchwald-Hartwig product)

SMILES: `c1ccc(Nc2ccccn2)cc1`

```
c1ccc(Nc2ccccn2)cc1
    → Brc1ccccn1 + Nc1ccccc1
    [buchwald_hartwig_retro]
```

- **2-Bromopyridine** (`Brc1ccccn1`) — in stock
- **Aniline** (`Nc1ccccc1`) — in stock

## 4-Fluorobiphenyl

SMILES: `Fc1ccc(-c2ccccc2)cc1`

RENKIN finds two Suzuki disconnection modes:

1. Fluorine-substituted arene as the bromide partner:
```
Fc1ccc(-c2ccccc2)cc1
    → Brc1ccc(F)cc1 + OB(O)c1ccccc1
```

2. Or the reverse:
```
Fc1ccc(-c2ccccc2)cc1
    → Brc1ccccc1 + OB(O)c1ccc(F)cc1
```

## Paracetamol (Acetaminophen)

SMILES: `CC(=O)Nc1ccc(O)cc1`

Amide bond disconnection:

```
CC(=O)Nc1ccc(O)cc1
    → CC(=O)O + Nc1ccc(O)cc1
    [amide_cleavage]
```

- **Acetic acid** (`CC(=O)O`) — in stock
- **4-Aminophenol** (`Nc1ccc(O)cc1`) — in stock

## Try More Examples

[**→ Open Playground**](../playground/){ .md-button }

Paste any SMILES into the playground to explore retrosynthetic routes.
