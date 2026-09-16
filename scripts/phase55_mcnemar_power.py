"""Plan an outcome-blind Phase 55.6 sample size for exact McNemar testing.

This is a protocol-registration tool, not a benchmark reporter.  It accepts
an alternative specified *before* TEST execution as the probabilities of the
two discordant paired outcomes: RENKIN-only and AiZynthFinder-only.  For each
candidate N it enumerates the paired multinomial distribution and calculates
the probability that the same two-sided exact McNemar test used by
``compare_paired_report.py`` rejects at the requested alpha.

The inputs must come from development-only evidence (for example the selected
VAL arm).  This tool deliberately cannot read TEST rows, preventing an
observed TEST outcome from selecting its own sample size.
"""

from __future__ import annotations

import argparse
from functools import lru_cache
import json
import math


PROTOCOL_VERSION = "renkin-phase55-mcnemar-power-v1"


def _probability(value: float, name: str) -> float:
    if not math.isfinite(value) or value < 0 or value > 1:
        raise ValueError(f"{name} must be a finite probability in [0, 1]")
    return value


def exact_mcnemar_p_value(renkin_only: int, aizynthfinder_only: int) -> float:
    """Return the two-sided exact p-value used in the paired report."""
    discordant = renkin_only + aizynthfinder_only
    if discordant == 0:
        return 1.0
    tail = sum(math.comb(discordant, count) for count in range(min(renkin_only, aizynthfinder_only) + 1))
    return min(1.0, 2.0 * tail / (2**discordant))


def _binomial_probability(trials: int, successes: int, probability: float) -> float:
    if probability == 0.0:
        return 1.0 if successes == 0 else 0.0
    if probability == 1.0:
        return 1.0 if successes == trials else 0.0
    log_value = (
        math.lgamma(trials + 1)
        - math.lgamma(successes + 1)
        - math.lgamma(trials - successes + 1)
        + successes * math.log(probability)
        + (trials - successes) * math.log1p(-probability)
    )
    return math.exp(log_value)


@lru_cache(maxsize=None)
def _conditional_rejection_probability(
    discordant: int, renkin_given_discordant: float, alpha: float
) -> float:
    """Return P(exact McNemar rejects | fixed discordant count).

    This value depends only on the registered alternative and the discordant
    count, not on the total sample size.  Planning scans many possible sample
    sizes, so caching it avoids repeatedly re-enumerating the same conditional
    binomial distribution without changing the exact test boundary.
    """
    return sum(
        _binomial_probability(discordant, renkin_only, renkin_given_discordant)
        for renkin_only in range(discordant + 1)
        if exact_mcnemar_p_value(renkin_only, discordant - renkin_only) <= alpha
    )


def exact_power(
    sample_size: int,
    renkin_only_probability: float,
    aizynthfinder_only_probability: float,
    alpha: float,
) -> float:
    """Compute rejection probability under a registered paired alternative."""
    if isinstance(sample_size, bool) or sample_size <= 0:
        raise ValueError("sample_size must be a positive integer")
    renkin_only_probability = _probability(renkin_only_probability, "renkin_only_probability")
    aizynthfinder_only_probability = _probability(aizynthfinder_only_probability, "aizynthfinder_only_probability")
    alpha = _probability(alpha, "alpha")
    discordant_probability = renkin_only_probability + aizynthfinder_only_probability
    if discordant_probability > 1.0:
        raise ValueError("discordant outcome probabilities must sum to at most 1")
    if discordant_probability == 0.0:
        return 0.0
    renkin_given_discordant = renkin_only_probability / discordant_probability
    power = 0.0
    for discordant in range(1, sample_size + 1):
        p_discordant = _binomial_probability(sample_size, discordant, discordant_probability)
        if p_discordant == 0.0:
            continue
        reject_given_discordant = _conditional_rejection_probability(
            discordant, renkin_given_discordant, alpha
        )
        power += p_discordant * reject_given_discordant
    return min(1.0, max(0.0, power))


def plan_sample_size(
    renkin_only_probability: float,
    aizynthfinder_only_probability: float,
    alpha: float,
    target_power: float,
    min_sample_size: int,
    max_sample_size: int,
) -> dict:
    target_power = _probability(target_power, "target_power")
    if target_power == 0.0:
        raise ValueError("target_power must be greater than zero")
    if min_sample_size <= 0 or max_sample_size < min_sample_size:
        raise ValueError("sample-size bounds must be positive and ordered")
    for sample_size in range(min_sample_size, max_sample_size + 1):
        achieved_power = exact_power(
            sample_size,
            renkin_only_probability,
            aizynthfinder_only_probability,
            alpha,
        )
        if achieved_power >= target_power:
            return {
                "protocol_version": PROTOCOL_VERSION,
                "eligible": True,
                "sample_size": sample_size,
                "achieved_power": achieved_power,
                "alpha": alpha,
                "target_power": target_power,
                "alternative": {
                    "renkin_only_probability": renkin_only_probability,
                    "aizynthfinder_only_probability": aizynthfinder_only_probability,
                    "absolute_rate_difference": renkin_only_probability - aizynthfinder_only_probability,
                    "discordant_probability": renkin_only_probability + aizynthfinder_only_probability,
                },
                "test": "two-sided exact McNemar",
                "input_boundary": "development-only assumptions; TEST rows are not an input",
            }
    return {
        "protocol_version": PROTOCOL_VERSION,
        "eligible": False,
        "reason": "target_power_not_reached_within_max_sample_size",
        "max_sample_size": max_sample_size,
        "achieved_power_at_max": exact_power(
            max_sample_size,
            renkin_only_probability,
            aizynthfinder_only_probability,
            alpha,
        ),
        "alpha": alpha,
        "target_power": target_power,
        "alternative": {
            "renkin_only_probability": renkin_only_probability,
            "aizynthfinder_only_probability": aizynthfinder_only_probability,
        },
        "test": "two-sided exact McNemar",
        "input_boundary": "development-only assumptions; TEST rows are not an input",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--renkin-only-probability", type=float, required=True)
    parser.add_argument("--aizynthfinder-only-probability", type=float, required=True)
    parser.add_argument("--alpha", type=float, default=0.05)
    parser.add_argument("--target-power", type=float, default=0.8)
    parser.add_argument("--min-sample-size", type=int, default=1)
    parser.add_argument("--max-sample-size", type=int, required=True)
    args = parser.parse_args()
    print(json.dumps(plan_sample_size(
        args.renkin_only_probability,
        args.aizynthfinder_only_probability,
        args.alpha,
        args.target_power,
        args.min_sample_size,
        args.max_sample_size,
    ), indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
