"""Paired statistics for the Issue #66 open-source planner comparison.

Since RENKIN and AiZynthFinder run on the SAME targets, every comparison is
paired -- bootstrap resamples whole target rows (both tools' outcomes for
that target together), never each tool's results independently, which would
throw away the pairing.

Stdlib only (random + math.comb) -- no numpy/scipy dependency for a handful
of percentile/binomial computations.

At n=100, every result here is explicitly DESCRIPTIVE ONLY: wide confidence
intervals are expected and must be shown, not narrated away. No
"statistically significant" claim is licensed at this sample size -- see
docs/guides/open-source-retrosynthesis-comparison.md, "Paired statistics".
"""

from __future__ import annotations

import math
import random
from dataclasses import dataclass

BOOTSTRAP_ITERATIONS = 10_000
BOOTSTRAP_SEED = 1066  # fixed, arbitrary -- a statistics-tool seed, not a
# retrosynthesis-tool seed; unlike AiZynthFinder's undocumented search
# randomness, this one is ours to fix and must never change between rounds.
CI_LEVEL = 0.95


@dataclass
class BootstrapResult:
    observed_diff: float
    ci_low: float
    ci_high: float
    n_iterations: int
    seed: int
    ci_level: float
    n_pairs: int


def paired_bootstrap_diff(
    pairs: list[tuple],
    statistic_fn,
    n_iterations: int = BOOTSTRAP_ITERATIONS,
    seed: int = BOOTSTRAP_SEED,
    ci_level: float = CI_LEVEL,
) -> BootstrapResult:
    """pairs: list of (value_a, value_b) tuples for the SAME target, in any order.

    statistic_fn(pairs) -> float: the statistic to bootstrap (e.g. a rate
    difference or mean difference). Called once on the observed data, then
    once per bootstrap resample of whole pairs (never resampling the two
    arms independently).
    """
    n = len(pairs)
    if n == 0:
        raise ValueError("cannot bootstrap zero pairs")

    rng = random.Random(seed)
    observed = statistic_fn(pairs)

    diffs = []
    for _ in range(n_iterations):
        resample = [pairs[rng.randrange(n)] for _ in range(n)]
        diffs.append(statistic_fn(resample))
    diffs.sort()

    alpha = (1 - ci_level) / 2
    lo_idx = max(0, int(alpha * n_iterations))
    hi_idx = min(n_iterations - 1, int((1 - alpha) * n_iterations) - 1)

    return BootstrapResult(
        observed_diff=observed,
        ci_low=diffs[lo_idx],
        ci_high=diffs[hi_idx],
        n_iterations=n_iterations,
        seed=seed,
        ci_level=ci_level,
        n_pairs=n,
    )


def rate_diff_statistic(pairs: list[tuple[bool | None, bool | None]]) -> float:
    """Mean(a) - mean(b), treating None (no measured outcome, e.g. timeout/
    crash) as False -- consistent with route_found_rate's all_sampled
    denominator convention (see PlannerComparisonRow schema doc)."""
    n = len(pairs)
    a_rate = sum(1 for a, _ in pairs if a is True) / n
    b_rate = sum(1 for _, b in pairs if b is True) / n
    return a_rate - b_rate


def mean_diff_statistic(pairs: list[tuple[float, float]]) -> float:
    """Mean(a) - mean(b) for numeric pairs (e.g. latency, memory). Callers
    must pre-filter to rows where both values are non-null (the
    measured_runs denominator) -- this function does not filter."""
    n = len(pairs)
    a_mean = sum(a for a, _ in pairs) / n
    b_mean = sum(b for _, b in pairs) / n
    return a_mean - b_mean


@dataclass
class McNemarResult:
    discordant_a_only: int  # a=True, b=False
    discordant_b_only: int  # a=False, b=True
    p_value: float


def mcnemar_exact(pairs: list[tuple[bool, bool]]) -> McNemarResult:
    """Exact (binomial-based) McNemar test on paired binary outcomes.
    Reference/optional statistic per the frozen protocol -- complements,
    never replaces, the bootstrap CI above."""
    b = sum(1 for a, c in pairs if a and not c)  # a-only
    c = sum(1 for a, c in pairs if c and not a)  # b-only
    n = b + c
    if n == 0:
        return McNemarResult(b, c, 1.0)

    k = min(b, c)
    # Two-sided exact test under Binomial(n, 0.5): P(X<=k) doubled, capped at 1.
    cdf_le_k = sum(math.comb(n, i) for i in range(0, k + 1)) / (2**n)
    p_value = min(1.0, 2 * cdf_le_k)
    return McNemarResult(b, c, p_value)


def percentile(values: list[float], p: float) -> float | None:
    """p in [0, 100]. Nearest-rank method; returns None for an empty list."""
    if not values:
        return None
    values_sorted = sorted(values)
    n = len(values_sorted)
    idx = min(n - 1, max(0, math.ceil(p / 100.0 * n) - 1))
    return values_sorted[idx]
