"""Conditional exact bounds for fixed finite edge populations.

These invert hypergeometric tails under ideal uniform sampling without
replacement and truthful labelled records. The portable hash sampler does not
prove that randomization assumption, semantic truth or review independence.
No design acceptance, noninferiority or joint qualification is issued here.
"""
from fractions import Fraction
from math import comb

from benchmarks.agent_query import broad_query_review as query


SCHEMA = "compass.broad-graph-conditional-bounds/1"
PROFILE = "exact-hypergeometric-bonferroni/1"
TOOLS = ("compass", "graphify")
MAX_ARITHMETIC_WORK = 8_192_000
MAX_FRAMES = 64
MAX_TOTAL_POPULATION = 8_000_000
MAX_TOTAL_SAMPLE = 65_536
FRAME_FIELDS = frozenset(("repository", "tool", "population", "sampleSize",
                          "correct", "incorrect", "buildSuccessful"))


class _Work:
    def __init__(self):
        self.used = 0

    def charge(self, amount):
        if self.used + amount > MAX_ARITHMETIC_WORK:
            raise ValueError("exact graph bounds arithmetic work limit exceeded")
        self.used += amount


def _range_sum(population, successes, sample, start, end, work):
    if start > end:
        return 0
    # Two initial binomial coefficients cost at most sample elementary factors.
    # Subsequent weights use an exact integer recurrence, not repeated combs.
    work.charge(sample + end - start + 1)
    weight = comb(successes, start) * comb(population - successes, sample - start)
    total = weight
    for observed in range(start, end):
        numerator = weight * (successes - observed) * (sample - observed)
        denominator = (observed + 1) * (population - successes - sample + observed + 1)
        weight, remainder = divmod(numerator, denominator)
        if remainder:
            raise ValueError("inexact hypergeometric weight recurrence")
        total += weight
    return total


def _tail(population, successes, sample, observed, denominator, upper, work):
    low = max(0, sample - population + successes)
    high = min(sample, successes)
    if observed < low:
        return denominator if upper else 0
    if observed > high:
        return 0 if upper else denominator
    left_end = observed - 1 if upper else observed
    left_size = left_end - low + 1
    right_size = high - left_end
    # Compute the shorter tail; subtracting integer counts loses no precision.
    if left_size <= right_size:
        left = _range_sum(population, successes, sample, low, left_end, work) if left_size else 0
        return denominator - left if upper else left
    right = _range_sum(population, successes, sample, left_end + 1, high, work)
    return right if upper else denominator - right


def count_bounds(population, sample, correct, incorrect, *, family_size=1, _work=None):
    """Return integer population-success endpoints, or None for an empty frame.

    Each of the family's two tails spends exactly 1/(40*family_size), for a
    simultaneous confidence level of at least19/20 under the stated model.
    Unreviewed sampled records are completed adversarially in both directions;
    no missing-at-random assumption or dropped denominator is used.
    """
    query._integer(population, "bound population", 0, 4_000_000)
    query._integer(sample, "bound sample", 0, min(population, 4096))
    query._integer(correct, "bound correct", 0, sample)
    query._integer(incorrect, "bound incorrect", 0, sample - correct)
    query._integer(family_size, "bound family size", 1, MAX_FRAMES)
    if not population:
        return None
    if sample == population:
        return correct, population - incorrect
    if not sample or (not correct and not incorrect):
        return 0, population
    work = _Work() if _work is None else _work
    work.charge(sample)
    denominator = comb(population, sample)
    alpha_denominator = 40 * family_size

    lower = 0
    if correct:
        low, high = correct, population - sample + correct
        while low < high:
            middle = (low + high) // 2
            accepted = _tail(population, middle, sample, correct, denominator, True, work)
            if accepted * alpha_denominator >= denominator:
                high = middle
            else:
                low = middle + 1
        lower = low

    upper = population
    if incorrect:
        observed = sample - incorrect
        low, high = observed, population - sample + observed
        while low < high:
            middle = (low + high + 1) // 2
            accepted = _tail(population, middle, sample, observed, denominator, False, work)
            if accepted * alpha_denominator >= denominator:
                low = middle
            else:
                high = middle - 1
        upper = low
    return lower, upper


def _ratio(value):
    return dict(numerator=value.numerator, denominator=value.denominator)


def _interval(lower, upper):
    return dict(lower=_ratio(lower), upper=_ratio(upper))


def population_bounds(frames):
    """Aggregate exact count bounds using captured population sizes, not quotas.

    A failed build withholds that tool's entire population interval. The
    difference uses simultaneous marginal endpoints; it assumes no pairing or
    independence between the tools. It is a conservative conditional bound,
    not a noninferiority decision or an accepted experimental design.
    """
    query._array(frames, "bound frames", MAX_FRAMES)
    checked, identities, repositories = [], set(), set()
    population_total, sample_total = 0, 0
    for raw in frames:
        frame = query._object(raw, "bound frame")
        if set(frame) != FRAME_FIELDS:
            raise ValueError("conditional bound frame fields differ from version1")
        repository = query._text(frame["repository"], "bound repository", 128)
        query._choice(frame["tool"], TOOLS, "bound tool")
        key = repository, frame["tool"]
        if key in identities:
            raise ValueError("duplicate conditional bound frame")
        identities.add(key)
        repositories.add(repository)
        n, sample = frame["population"], frame["sampleSize"]
        query._integer(n, "bound population", 0, 4_000_000)
        query._integer(sample, "bound sample", 0, min(n, 4096))
        query._integer(frame["correct"], "bound correct", 0, sample)
        query._integer(frame["incorrect"], "bound incorrect", 0, sample - frame["correct"])
        query._boolean(frame["buildSuccessful"], "bound build status")
        population_total += n
        sample_total += sample
        if population_total > MAX_TOTAL_POPULATION or sample_total > MAX_TOTAL_SAMPLE:
            raise ValueError("aggregate conditional bound population/sample limit exceeded")
        checked.append(dict(frame))
    if identities != {(repository, tool) for repository in repositories for tool in TOOLS}:
        raise ValueError("conditional bounds require both tool frames for each repository")
    checked.sort(key=lambda frame: (frame["repository"], frame["tool"]))
    family_size = sum(frame["population"] > 0 for frame in checked)
    work, counts, by_frame = _Work(), {}, []
    for frame in checked:
        result = None
        if frame["buildSuccessful"]:
            result = count_bounds(frame["population"], frame["sampleSize"],
                frame["correct"], frame["incorrect"], family_size=max(1, family_size), _work=work)
        counts[frame["repository"], frame["tool"]] = result
        by_frame.append(dict(repository=frame["repository"], tool=frame["tool"],
            lowerCount=result[0] if result is not None else None,
            upperCount=result[1] if result is not None else None))
    tools, exact = {}, {}
    for tool in TOOLS:
        selected = [frame for frame in checked if frame["tool"] == tool]
        population = sum(frame["population"] for frame in selected)
        interval = None
        if population and all(frame["buildSuccessful"] for frame in selected):
            lower = sum(counts[frame["repository"], tool][0] for frame in selected if frame["population"])
            upper = sum(counts[frame["repository"], tool][1] for frame in selected if frame["population"])
            interval = Fraction(lower, population), Fraction(upper, population)
        exact[tool] = interval
        tools[tool] = _interval(*interval) if interval is not None else None
    difference = None
    if all(exact[tool] is not None for tool in TOOLS):
        candidate, comparator = (exact[tool] for tool in TOOLS)
        difference = _interval(candidate[0] - comparator[1], candidate[1] - comparator[0])
    return dict(schema=SCHEMA, profile=PROFILE, familyConfidence=_ratio(Fraction(19, 20)),
        familySize=family_size, frames=by_frame, tools=tools, difference=difference,
        arithmeticWork=work.used, arithmeticWorkLimit=MAX_ARITHMETIC_WORK,
        qualificationEstablished=False, registeredDesignAccepted=False,
        semanticTruthEstablished=False, independenceEstablished=False,
        conditionalOn="ideal uniform sampling without replacement and truthful labelled records",
        limitations=["Hash-stream reproducibility does not prove ideal randomization or independent review.",
            "Counts describe emitted records in the captured frames, not source-fact recall or future repositories.",
            "Bonferroni aggregation is conservative; power and the final joint decision remain separate."])
