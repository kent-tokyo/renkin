"""Common, tool-agnostic route DAG representation.

Both RENKIN's native route JSON (a flat `steps` list joined by SMILES
equality, plus a pre-computed `building_blocks` leaf list) and
AiZynthFinder's exported route tree (nested mol/reaction dicts with an
`in_stock` flag) normalize into this one representation, so
`normalized_route_sha256` means the same thing regardless of source tool.

Canonicalizer: RDKit, applied uniformly to every SMILES from either tool --
never chematic's `canonical_smiles`, which is a documented non-invariant
stable fixed point (see docs/guides/open-source-retrosynthesis-comparison.md,
"Canonicalizer choice") that would bias leaf-matching toward RENKIN's own
notation lineage.
"""

from __future__ import annotations

import hashlib
import json
from dataclasses import dataclass, field

try:
    from rdkit import Chem, RDLogger

    RDLogger.DisableLog("rdApp.*")
    HAVE_RDKIT = True
except ImportError:  # pragma: no cover -- exercised by scripts/tests without the dep installed
    HAVE_RDKIT = False

# Closed taxonomy -- see docs/guides/open-source-retrosynthesis-comparison.md
# "Common structural warning taxonomy". Values used as both parse-failure
# reasons and (for the informational subset) common_validation_warnings codes.
RAW_OUTPUT_NOT_DECODABLE = "raw_output_not_decodable"
MULTIPLE_OR_ZERO_ROOTS = "multiple_or_zero_roots"
ROOT_MISMATCH = "root_mismatch"
CYCLE_DETECTED = "cycle_detected"
DISCONNECTED_REFERENCE = "disconnected_reference"
UNPARSEABLE_SMILES_IN_ROUTE = "unparseable_smiles_in_route"
CHILDLESS_NON_LEAF = "childless_non_leaf"
AMBIGUOUS_LEAF_STATUS = "ambiguous_leaf_status"
DEGENERATE_SELF_REFERENTIAL_STEP = "degenerate_self_referential_step"
STEP_ARITY_MISMATCH = "step_arity_mismatch"


def canonicalize(raw_smiles: str) -> str | None:
    if not HAVE_RDKIT:
        raise RuntimeError(
            "rdkit is required (pip install -r scripts/requirements-compare-66.txt)"
        )
    mol = Chem.MolFromSmiles(raw_smiles)
    if mol is None:
        return None
    return Chem.MolToSmiles(mol, canonical=True)


@dataclass
class RouteNode:
    canonical_smiles: str
    # True = tool claims this is a resolved stock leaf. False = tool
    # explicitly flags it unresolved (not expanded further, not in stock).
    # None = leaf with no explicit flag either way -- itself a defect
    # (ambiguous_leaf_status), never silently defaulted to True or False.
    is_stock_leaf: bool | None
    children: list["RouteNode"] = field(default_factory=list)


@dataclass
class RouteGraph:
    root: RouteNode
    # Number of parent->children edges after normalization (one per
    # disconnection step) -- distinct from any tool-reported step count.
    step_count_collapsed_edges: int


@dataclass
class ParseOutcome:
    graph: RouteGraph | None
    parseable: bool
    defects: list[str] = field(default_factory=list)


def _count_edges(node: RouteNode) -> int:
    total = 1 if node.children else 0
    for c in node.children:
        total += _count_edges(c)
    return total


def normalize_renkin_route(route: dict, requested_target_smiles: str) -> ParseOutcome:
    """route: one entry of the CLI's `routes` array (has `steps`, `building_blocks`)."""
    defects: list[str] = []
    try:
        steps = route["steps"]
        building_blocks = route["building_blocks"]
    except (KeyError, TypeError):
        return ParseOutcome(None, False, [RAW_OUTPUT_NOT_DECODABLE])

    if not steps:
        return ParseOutcome(None, False, [MULTIPLE_OR_ZERO_ROOTS])

    requested_canon = canonicalize(requested_target_smiles)
    if requested_canon is None:
        return ParseOutcome(None, False, [UNPARSEABLE_SMILES_IN_ROUTE])

    bb_canon = set()
    for bb in building_blocks:
        c = canonicalize(bb)
        if c is None:
            defects.append(UNPARSEABLE_SMILES_IN_ROUTE)
        else:
            bb_canon.add(c)

    # target (canonical) -> step dict, detect duplicate targets (ambiguous graph)
    steps_by_target: dict[str, dict] = {}
    for step in steps:
        try:
            target_raw = step["target"]
            precursors_raw = step["precursors"]
        except (KeyError, TypeError):
            defects.append(RAW_OUTPUT_NOT_DECODABLE)
            continue
        canon = canonicalize(target_raw)
        if canon is None:
            defects.append(UNPARSEABLE_SMILES_IN_ROUTE)
            continue
        steps_by_target[canon] = {"precursors": precursors_raw}

    root_canon = canonicalize(steps[0]["target"]) if steps else None
    if root_canon is None or root_canon != requested_canon:
        defects.append(ROOT_MISMATCH)

    on_stack: set[str] = set()

    def build(canon_smiles: str) -> RouteNode:
        if canon_smiles in on_stack:
            defects.append(CYCLE_DETECTED)
            return RouteNode(canon_smiles, is_stock_leaf=None, children=[])
        step = steps_by_target.get(canon_smiles)
        if step is None:
            # Not any step's target -> a leaf. RENKIN's own building_blocks
            # list is exactly this set, computed by the engine itself, so
            # leaf status is unambiguous by construction for RENKIN routes.
            is_leaf_in_stock = canon_smiles in bb_canon
            if not is_leaf_in_stock:
                # A precursor that's neither a step target nor a declared
                # building block -- RENKIN's own invariant is broken.
                defects.append(AMBIGUOUS_LEAF_STATUS)
                return RouteNode(canon_smiles, is_stock_leaf=None, children=[])
            return RouteNode(canon_smiles, is_stock_leaf=True, children=[])

        on_stack.add(canon_smiles)
        children = []
        for precursor_raw in step["precursors"]:
            if canonicalize(precursor_raw) == canon_smiles:
                defects.append(DEGENERATE_SELF_REFERENTIAL_STEP)
                continue
            p_canon = canonicalize(precursor_raw)
            if p_canon is None:
                defects.append(UNPARSEABLE_SMILES_IN_ROUTE)
                continue
            children.append(build(p_canon))
        on_stack.discard(canon_smiles)
        if not children:
            defects.append(CHILDLESS_NON_LEAF)
        return RouteNode(canon_smiles, is_stock_leaf=False, children=children)

    root_node = build(root_canon if root_canon is not None else requested_canon)

    parseable = len(defects) == 0
    graph = RouteGraph(root=root_node, step_count_collapsed_edges=_count_edges(root_node))
    return ParseOutcome(graph if parseable else None, parseable, defects)


def normalize_aizynthfinder_route(tree: dict, requested_target_smiles: str) -> ParseOutcome:
    """tree: one entry of aizynthcli's `trees` column for a target.

    Defensive: AiZynthFinder's route dict interposes explicit "reaction"
    nodes between "mol" nodes (mol -> reaction -> mol -> ...). Any node
    missing the expected discriminator/fields is a parse failure, never a
    best-effort guess.
    """
    defects: list[str] = []
    if not isinstance(tree, dict):
        return ParseOutcome(None, False, [RAW_OUTPUT_NOT_DECODABLE])

    requested_canon = canonicalize(requested_target_smiles)
    if requested_canon is None:
        return ParseOutcome(None, False, [UNPARSEABLE_SMILES_IN_ROUTE])

    on_stack: set[int] = set()

    def build_mol(node: dict, depth: int) -> RouteNode | None:
        if node.get("type") != "mol":
            defects.append(RAW_OUTPUT_NOT_DECODABLE)
            return None
        raw_smiles = node.get("smiles")
        if raw_smiles is None:
            defects.append(RAW_OUTPUT_NOT_DECODABLE)
            return None
        canon = canonicalize(raw_smiles)
        if canon is None:
            defects.append(UNPARSEABLE_SMILES_IN_ROUTE)
            return None

        node_key = id(node)
        if node_key in on_stack:
            defects.append(CYCLE_DETECTED)
            return RouteNode(canon, is_stock_leaf=None, children=[])

        reaction_children = node.get("children") or []
        if not reaction_children:
            in_stock = node.get("in_stock")
            if in_stock is None:
                defects.append(AMBIGUOUS_LEAF_STATUS)
                return RouteNode(canon, is_stock_leaf=None, children=[])
            return RouteNode(canon, is_stock_leaf=bool(in_stock), children=[])

        on_stack.add(node_key)
        children: list[RouteNode] = []
        for reaction_node in reaction_children:
            if not isinstance(reaction_node, dict) or reaction_node.get("type") != "reaction":
                defects.append(RAW_OUTPUT_NOT_DECODABLE)
                continue
            mol_children = reaction_node.get("children") or []
            for mol_child in mol_children:
                child_node = build_mol(mol_child, depth + 1)
                if child_node is None:
                    continue
                if child_node.canonical_smiles == canon:
                    defects.append(DEGENERATE_SELF_REFERENTIAL_STEP)
                    continue
                children.append(child_node)
        on_stack.discard(node_key)
        if not children:
            defects.append(CHILDLESS_NON_LEAF)
        return RouteNode(canon, is_stock_leaf=False, children=children)

    root_node = build_mol(tree, 0)
    if root_node is None:
        return ParseOutcome(None, False, defects or [RAW_OUTPUT_NOT_DECODABLE])
    if root_node.canonical_smiles != requested_canon:
        defects.append(ROOT_MISMATCH)

    parseable = len(defects) == 0
    graph = RouteGraph(root=root_node, step_count_collapsed_edges=_count_edges(root_node))
    return ParseOutcome(graph if parseable else None, parseable, defects)


def normalized_route_sha256(graph: RouteGraph) -> str:
    """Inclusion-list hash: (canonical_smiles, is_stock_leaf, n_children) per
    node, pre-order, children sorted ascending by canonical_smiles. Excludes
    everything tool-specific (timing, scores, template ids, ...) by
    construction -- only what's listed here is ever fed into the hash.
    """

    def emit(node: RouteNode) -> list:
        children_sorted = sorted(node.children, key=lambda c: c.canonical_smiles)
        return [
            node.canonical_smiles,
            node.is_stock_leaf,
            len(node.children),
            [emit(c) for c in children_sorted],
        ]

    payload = {"schema": "renkin-issue66-route-hash-v1", "tree": emit(graph.root)}
    text = json.dumps(payload, sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def count_leaves(node: RouteNode) -> int:
    if not node.children:
        return 1
    return sum(count_leaves(c) for c in node.children)


def graph_depth(node: RouteNode) -> int:
    """Longest root-to-leaf path length, in edges. Used as a harness-derived
    fallback for `best_route_depth` when a tool's own reported depth field
    isn't reliably identifiable (see the AiZynthFinder adapter)."""
    if not node.children:
        return 0
    return 1 + max(graph_depth(c) for c in node.children)


def iter_leaves(node: RouteNode):
    if not node.children:
        yield node
    else:
        for c in node.children:
            yield from iter_leaves(c)
