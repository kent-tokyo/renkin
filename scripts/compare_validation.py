"""Common, tool-agnostic post-hoc route validation.

Operates ONLY on the normalized RouteGraph (compare_route_graph.py) -- never
on a tool's native output shape directly, so this logic runs identically
regardless of which tool produced the route.

Caveat that must accompany every metric this module produces (see
docs/guides/open-source-retrosynthesis-comparison.md, "What this validation
does not claim"): atom-balanced does not mean chemically correct; canonical-
SMILES leaf matching does not account for tautomers or differing
stereochemistry conventions; no route here has been reviewed by a human
chemist; tool-native "solved" and this module's post-hoc "accepted" are
always reported as separate metrics.
"""

from __future__ import annotations

from collections import Counter
from dataclasses import dataclass, field

from compare_route_graph import (
    RouteGraph,
    RouteNode,
    canonicalize,
    iter_leaves,
)

try:
    from rdkit import Chem

    HAVE_RDKIT = True
except ImportError:  # pragma: no cover
    HAVE_RDKIT = False

LEAF_CLAIMED_STOCK_NOT_MATCHED = "leaf_claimed_stock_not_matched"
LEAF_UNRESOLVED = "leaf_unresolved"
UNACCOUNTED_TARGET_ELEMENT = "unaccounted_target_element"
CHARGE_IMBALANCE = "charge_imbalance"  # informational only, never gates
STEREO_CENTER_COUNT_MISMATCH = "stereo_center_count_mismatch"  # informational only
DUPLICATE_ROUTE_WITHIN_TARGET = "duplicate_route_within_target"  # informational, target-level

CAVEAT_TEXT = (
    "Target-element-accounted (target_element_accounting_status) means that, for every "
    "step, the target's heavy-atom count per element does not exceed the sum over all "
    "precursors -- a directional atom-accounting inequality, not exact mass conservation "
    "(precursors may legitimately carry MORE atoms than the target; that excess is an "
    "untracked byproduct, not a failure). "
    "It is not validated against real reaction feasibility, mechanism, or literature "
    "precedent, and must never be read as \"chemically correct\" or \"chemically valid\". "
    "All-leaves-in-configured-stock is an exact canonical-SMILES string match against the "
    "stock actually configured for that run; it does not account for tautomers, and does "
    "not account for the two tools' potentially differing stereochemistry conventions -- a "
    "leaf that is the same molecule as a stock entry in every chemically meaningful sense "
    "can still be reported as missing if its SMILES notation diverges in either of those "
    "ways. No route in this benchmark has been reviewed by a human chemist, and no "
    "accuracy or correctness claim about any individual route, or about either tool's "
    "routes in aggregate, is licensed by this harness alone. Tool-native \"solved\" and "
    "this harness's post-hoc \"accepted\" are reported as separate metrics and must never "
    "be merged into one number."
)


@dataclass
class LeafOutcome:
    canonical_smiles: str
    tool_claimed_stock: bool | None
    matched: bool | None  # None only when tool_claimed_stock is False (leaf_unresolved)
    outcome: str  # "matched" | LEAF_CLAIMED_STOCK_NOT_MATCHED | LEAF_UNRESOLVED


@dataclass
class StockValidationResult:
    all_leaves_in_configured_stock: bool
    leaf_breakdown: list[LeafOutcome] = field(default_factory=list)


def build_stock_set(stock_smiles: list[str]) -> set[str]:
    canon = set()
    for s in stock_smiles:
        c = canonicalize(s)
        if c is not None:
            canon.add(c)
    return canon


def validate_stock_leaves(graph: RouteGraph, configured_stock_canon: set[str]) -> StockValidationResult:
    breakdown: list[LeafOutcome] = []
    all_ok = True
    for leaf in iter_leaves(graph.root):
        if leaf.is_stock_leaf is True:
            if leaf.canonical_smiles in configured_stock_canon:
                breakdown.append(
                    LeafOutcome(leaf.canonical_smiles, True, True, "matched")
                )
            else:
                breakdown.append(
                    LeafOutcome(
                        leaf.canonical_smiles, True, False, LEAF_CLAIMED_STOCK_NOT_MATCHED
                    )
                )
                all_ok = False
        elif leaf.is_stock_leaf is False:
            breakdown.append(LeafOutcome(leaf.canonical_smiles, False, None, LEAF_UNRESOLVED))
            all_ok = False
        else:
            # Ambiguous leaf status should already have failed route_tree_parseable
            # upstream; treat defensively as not-ok if reached here anyway.
            breakdown.append(LeafOutcome(leaf.canonical_smiles, None, None, LEAF_UNRESOLVED))
            all_ok = False
    return StockValidationResult(all_ok, breakdown)


def _heavy_atom_counts(canonical_smiles: str) -> Counter | None:
    if not HAVE_RDKIT:
        raise RuntimeError("rdkit is required")
    mol = Chem.MolFromSmiles(canonical_smiles)
    if mol is None:
        return None
    counts: Counter = Counter()
    for atom in mol.GetAtoms():
        if atom.GetSymbol() != "H":
            counts[atom.GetSymbol()] += 1
    return counts


def _net_charge(canonical_smiles: str) -> int | None:
    if not HAVE_RDKIT:
        raise RuntimeError("rdkit is required")
    mol = Chem.MolFromSmiles(canonical_smiles)
    if mol is None:
        return None
    return sum(atom.GetFormalCharge() for atom in mol.GetAtoms())


def _stereo_center_count(canonical_smiles: str) -> int | None:
    if not HAVE_RDKIT:
        raise RuntimeError("rdkit is required")
    mol = Chem.MolFromSmiles(canonical_smiles)
    if mol is None:
        return None
    return sum(
        1 for atom in mol.GetAtoms() if atom.GetChiralTag() != Chem.ChiralType.CHI_UNSPECIFIED
    )


@dataclass
class StepArityInfo:
    step_count_collapsed_edges: int
    warnings: list[str] = field(default_factory=list)


def check_reaction_steps_parseable(graph: RouteGraph) -> tuple[bool, list[str]]:
    """Per-edge check: reactant/product SMILES independently parseable (already
    true post-normalization) and no residual self-loop. Precondition:
    route_tree_parseable must already be true -- callers should not invoke this
    otherwise (result would vacuously be (True, []) with no edges to check).
    """
    warnings: list[str] = []

    def walk(node: RouteNode) -> bool:
        ok = True
        for child in node.children:
            if child.canonical_smiles == node.canonical_smiles:
                ok = False
            ok = walk(child) and ok
        return ok

    ok = walk(graph.root)
    return ok, warnings


def check_target_element_accounting(graph: RouteGraph) -> tuple[str, list[str]]:
    """Directional per-element check: for every step, target's heavy-atom
    count per element must be <= the SUM over all precursors (precursors may
    legitimately carry MORE atoms -- the excess is an untracked byproduct;
    the target must never have atoms the precursor pool can't account for).

    This is NOT exact mass conservation -- it is a one-directional inequality
    that only ever flags a target with MORE of an element than its precursors
    can supply. Returns (status, warning_codes) where status in
    {"accounted", "unaccounted_target_element", "not_evaluable"}.
    """
    warnings: list[str] = []
    any_evaluated = False
    unaccounted = False

    def walk(node: RouteNode) -> None:
        nonlocal any_evaluated, unaccounted
        if not node.children:
            return
        target_counts = _heavy_atom_counts(node.canonical_smiles)
        precursor_counts: Counter = Counter()
        countable = target_counts is not None
        for child in node.children:
            c = _heavy_atom_counts(child.canonical_smiles)
            if c is None:
                countable = False
            else:
                precursor_counts.update(c)
        if countable:
            any_evaluated = True
            elements_in_excess = [
                el for el, n in target_counts.items() if n > precursor_counts.get(el, 0)
            ]
            if elements_in_excess:
                unaccounted = True
                warnings.append(UNACCOUNTED_TARGET_ELEMENT)

            target_charge = _net_charge(node.canonical_smiles)
            precursor_charge = sum(
                (_net_charge(c.canonical_smiles) or 0) for c in node.children
            )
            if target_charge is not None and target_charge != precursor_charge:
                warnings.append(CHARGE_IMBALANCE)

            target_stereo = _stereo_center_count(node.canonical_smiles)
            precursor_stereo = sum(
                (_stereo_center_count(c.canonical_smiles) or 0) for c in node.children
            )
            if target_stereo is not None and target_stereo != precursor_stereo:
                warnings.append(STEREO_CENTER_COUNT_MISMATCH)

        for child in node.children:
            walk(child)

    walk(graph.root)

    if not any_evaluated:
        return "not_evaluable", warnings
    return ("unaccounted_target_element" if unaccounted else "accounted"), warnings
