"""Source-fact scoring for Plan 029; an offline auditor, never a graph producer.

Collection reuses runner.run_bounded. A reviewed, typed normalization connects
captured output spans to declarations/facts; prose substrings are not an oracle.
Checks establish artifact integrity and the registered arithmetic, not the
truthfulness or independence of a source-review attestation.
"""
from __future__ import annotations

import argparse
from collections import Counter, defaultdict
from dataclasses import dataclass
from datetime import datetime
import hashlib
import json
import math
from pathlib import Path, PurePosixPath
import random
import re
from statistics import NormalDist, mean
import sys
from typing import Any
from urllib.parse import urlsplit

from benchmarks.agent_query.community_tasks import read_bounded
from benchmarks.agent_query.path_audit import _edge_site
from benchmarks.agent_query.runner import _node_anchor

REGISTRATION_SCHEMA = "compass.broad-query-registration/1"
RUN_SCHEMA = "compass.broad-query-capture/1"
NORMALIZED_SCHEMA = "compass.broad-query-normalized/1"
REVIEW_SCHEMA = "compass.broad-query-judgments/1"
MANIFEST_SCHEMA = "compass.broad-query-review-input/1"
REPORT_SCHEMA = "compass.broad-query-review/1"
MAX_JSON_BYTES = 64 * 1024 * 1024
MAX_GRAPH_BYTES = 256 * 1024 * 1024
MAX_SOURCE_BYTES = 4 * 1024 * 1024
MAX_TASKS = 2048
MAX_CANDIDATES = 512
MAX_CLAIMS = 128
TOOLS = ("compass", "graphify")
ARMS = ("default", "normalized")
KINDS = ("broad", "exact", "directed", "multihop", "ambiguous", "unsupported")
SHA = re.compile(r"[0-9a-f]{64}\Z")


def _object(value: Any, where: str) -> dict:
    if not isinstance(value, dict):
        raise ValueError(f"{where} must be an object")
    return value


def _array(value: Any, where: str, maximum: int, minimum: int = 0) -> list:
    if not isinstance(value, list) or not minimum <= len(value) <= maximum:
        raise ValueError(f"{where} must contain {minimum}..{maximum} entries")
    return value


def _text(value: Any, where: str, maximum: int = 4096, *, empty: bool = False,
          allow_tabs: bool = False) -> str:
    if not isinstance(value, str) or (not value and not empty) or len(value.encode()) > maximum:
        raise ValueError(f"invalid {where}")
    if any(ord(c) < 32 and not (allow_tabs and c == "\t") for c in value):
        raise ValueError(f"control character in {where}")
    return value


def _integer(value: Any, where: str, low: int = 0, high: int = 10**12) -> int:
    if type(value) is not int or not low <= value <= high:
        raise ValueError(f"{where} must be an integer in {low}..{high}")
    return value


def _number(value: Any, where: str, low: float, high: float) -> float:
    if type(value) not in (int, float) or not math.isfinite(value) or not low <= value <= high:
        raise ValueError(f"invalid finite {where}")
    return float(value)


def _boolean(value: Any, where: str) -> bool:
    if type(value) is not bool:
        raise ValueError(f"{where} must be boolean")
    return value


def _choice(value: Any, choices, where: str) -> str:
    if value not in choices:
        raise ValueError(f"unsupported {where}")
    return value


def _unique(values: list, where: str) -> None:
    if len(values) != len(set(values)):
        raise ValueError(f"duplicate {where}")


def _digest(value: Any, where: str = "SHA-256") -> str:
    if not isinstance(value, str) or not SHA.fullmatch(value):
        raise ValueError(f"invalid {where}")
    return value


def _relative(value: Any) -> str:
    value = _text(value, "repository-relative file", 1024)
    path = PurePosixPath(value)
    if path.is_absolute() or ".." in path.parts or "\\" in value or str(path) != value:
        raise ValueError("file must be a canonical relative POSIX path")
    return value


def _timestamp(value: Any) -> datetime:
    value = _text(value, "UTC timestamp", 40)
    try:
        result = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise ValueError("invalid UTC timestamp") from error
    if result.tzinfo is None or result.utcoffset().total_seconds() != 0:
        raise ValueError("timestamp must explicitly use UTC")
    return result


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _json(data: bytes) -> dict:
    def pairs(items):
        result = {}
        for key, value in items:
            if key in result:
                raise ValueError(f"duplicate JSON key {key!r}")
            result[key] = value
        return result
    try:
        return _object(json.loads(data, object_pairs_hook=pairs,
                                  parse_constant=lambda _: (_ for _ in ()).throw(
                                      ValueError("nonfinite JSON number"))), "document")
    except (UnicodeDecodeError, RecursionError) as error:
        raise ValueError("invalid or excessively nested JSON") from error


@dataclass(frozen=True, order=True)
class Identity:
    file: str
    start_line: int
    symbol: str
    owner: str

    @classmethod
    def parse(cls, raw: Any) -> Identity:
        raw = _object(raw, "declaration identity")
        if set(raw) != {"file", "startLine", "symbol", "owner"}:
            raise ValueError("identity needs exactly file/startLine/symbol/owner")
        return cls(_relative(raw["file"]), _integer(raw["startLine"], "startLine", 1),
                   _text(raw["symbol"], "terminal symbol", 512),
                   _text(raw["owner"], "lexical owner", 1024, empty=True))

    def record(self) -> dict:
        return dict(file=self.file, startLine=self.start_line, symbol=self.symbol, owner=self.owner)


def _site(raw: Any) -> tuple[str, int, str]:
    raw = _object(raw, "source witness")
    return (_relative(raw.get("file")), _integer(raw.get("line"), "source line", 1),
            _text(raw.get("text"), "source witness text", allow_tabs=True))


def _relation(raw: Any, *, observed: bool = False) -> tuple | None:
    if raw is None:
        return None
    raw = _object(raw, "relationship fact")
    sites = [_site(s) for s in _array(raw.get("occurrences"), "occurrences", 64, 0 if observed else 1)]
    _unique([(f, n) for f, n, _ in sites], "source call occurrences")
    return (Identity.parse(raw.get("source")), Identity.parse(raw.get("target")),
            _text(raw.get("kind"), "relationship kind", 128),
            _choice(raw.get("direction"), ("source_to_target",), "relationship direction"),
            tuple(sorted(sites)))


def _relation_matches(fact: dict, observed: tuple | None) -> bool:
    """Match only facts actually printed; provenance sites are never backfilled.

    Site/count requests require exact frozen line witnesses. Connectivity-only
    questions may omit sites, but any sites actually asserted must be supported.
    Line witnesses alone do not establish column-distinct call multiplicity.
    """
    expected = _relation(fact.get("relation"))
    if expected is None or observed is None:
        return expected is observed
    if expected[:4] != observed[:4]:
        return False
    if fact.get("requireOccurrences", True):
        return expected[4] == observed[4]
    return set(observed[4]) <= set(expected[4])


def validate_registration(raw: dict) -> dict:
    """Validate source registrations, including explicitly pending AI-authored ones.

    Validation deliberately cannot upgrade a pending review to qualification.
    """
    raw = _object(raw, "registration")
    if raw.get("schema") != REGISTRATION_SCHEMA:
        raise ValueError("unsupported broad-query registration schema")
    _choice(raw.get("designation"), ("development", "heldout"), "designation")
    _timestamp(raw.get("frozenAt"))
    repos = _array(raw.get("repositories"), "repositories", 32, 1)
    repo_ids, file_sets = [], {}
    for repo in repos:
        repo = _object(repo, "repository")
        name = _text(repo.get("id"), "repository id", 128)
        repo_ids.append(name)
        _text(repo.get("language"), "language", 64)
        _text(repo.get("url"), "source URL", 2048)
        if not re.fullmatch(r"[0-9a-f]{40,64}", str(repo.get("commit", ""))):
            raise ValueError("repository commit must be a full hexadecimal object id")
        files = _array(repo.get("files"), "registered source files", 4096, 1)
        names = []
        for entry in files:
            names.append(_relative(entry.get("file")))
            _digest(entry.get("sha256"), "source-file SHA-256")
        _unique(names, "source files")
        file_sets[name] = set(names)
    _unique(repo_ids, "repositories")
    tasks = _array(raw.get("tasks"), "tasks", MAX_TASKS, 1)
    task_ids = []
    for task in tasks:
        task = _object(task, "task")
        task_ids.append(_text(task.get("id"), "task id", 128))
        repo = _choice(task.get("repository"), repo_ids, "task repository")
        _text(task.get("cluster"), "independent task cluster", 128)
        kind = _choice(task.get("kind"), KINDS, "task kind")
        _text(task.get("question"), "question")
        expected = _choice(task.get("expected"), ("answer", "ambiguous", "abstain"), "expected disposition")
        relevant = _array(task.get("relevant"), "relevance judgments", MAX_CANDIDATES)
        identities = []
        positive = []
        for judgment in relevant:
            identity = Identity.parse(judgment.get("identity"))
            if identity.file not in file_sets[repo]:
                raise ValueError("relevant identity references an unpinned source file")
            identities.append(identity)
            grade = _integer(judgment.get("grade"), "relevance grade", 0, 3)
            if grade >= 2:
                positive.append(identity)
        _unique(identities, "relevance identities")
        facts = _array(task.get("facts"), "source facts", 64)
        fact_ids = []
        for fact in facts:
            fact_ids.append(_text(fact.get("id"), "fact id", 128))
            identity = Identity.parse(fact.get("identity"))
            if identity not in positive:
                raise ValueError("source fact owner must have a frozen positive relevance judgment")
            _boolean(fact.get("essential"), "essential fact")
            sites = [_site(fact.get("source"))]
            relation = _relation(fact.get("relation"))
            if relation:
                _boolean(fact.get("requireOccurrences", True), "required answer occurrences")
                sites.extend(relation[4])
                if relation[0].file not in file_sets[repo] or relation[1].file not in file_sets[repo]:
                    raise ValueError("relationship endpoints reference unpinned files")
            if any(f not in file_sets[repo] for f, _, _ in sites):
                raise ValueError("source witness references an unpinned file")
        _unique(fact_ids, "fact ids")
        if expected == "answer" and (not positive or not any(f["essential"] for f in facts)):
            raise ValueError("answerable task needs relevant declarations and essential facts")
        if expected == "ambiguous" and len(positive) < 2:
            raise ValueError("ambiguity requires at least two independently reviewed identities")
        if expected == "abstain":
            if relevant or facts or kind != "unsupported":
                raise ValueError("unsupported tasks cannot contain positive facts or candidates")
            _text(task.get("rationale"), "scoped no-support rationale")
        if (kind == "ambiguous") != (expected == "ambiguous"):
            raise ValueError("ambiguous task kind and disposition disagree")
    _unique(task_ids, "task ids")
    if "exposureLineage" in raw:
        # Shape validation admits pending independent review. Actual capture
        # and qualification require the returned failures to be empty.
        exposure_failures(raw)
    graph_scope = raw.get("graphQualificationRepositories")
    if graph_scope is not None:
        graph_scope = _array(graph_scope, "graph quality claim repository scope", 32, 8)
        _unique(graph_scope, "graph quality claim repositories")
        if any(repo not in repo_ids for repo in graph_scope):
            raise ValueError("graph quality claim scope has an unregistered repository")
    cohorts = raw.get("metricCohorts")
    if cohorts is not None:
        _object(cohorts, "prospectively frozen metric cohorts")
        if set(cohorts) != {"broadSuccess", "exactDirected", "claimPrecisionEmptyZero"}:
            raise ValueError("all three predeclared metric cohorts are required")
        task_map = {t["id"]: t for t in tasks}
        for metric, cohort in cohorts.items():
            selected = _array(cohort.get("taskIds"), "metric cohort tasks", MAX_TASKS, 1)
            _unique(selected, "metric cohort tasks")
            _text(cohort.get("rationale"), "prospective cohort rationale", 16384)
            _integer(cohort.get("minimumRepositories"), "cohort minimum repositories", 8, 32)
            _integer(cohort.get("minimumIndependentClusters"), "cohort minimum independent clusters", 80 if metric == "broadSuccess" else 1, MAX_TASKS)
            allowed = ("exact", "directed") if metric == "exactDirected" else ("broad",)
            if any(t not in task_map or task_map[t]["kind"] not in allowed for t in selected):
                raise ValueError("metric cohort contains unknown or incorrectly typed tasks")
        if set(cohorts["broadSuccess"]["taskIds"]) != set(cohorts["claimPrecisionEmptyZero"]["taskIds"]):
            raise ValueError("broad source-claim precision must use the frozen broad comparison cohort")
        _text(raw.get("samplingDesign"), "source-first sampling and original-panel retention", 16384)
        broad_repos = {task_map[t]["repository"] for t in cohorts["broadSuccess"]["taskIds"]}
        if graph_scope is not None and not broad_repos <= set(graph_scope):
            raise ValueError("graph quality claim scope must cover every broad comparison repository")
    return raw


class GraphIndex:
    """Exact anchored identities; graph counts never become a quality oracle."""

    def __init__(self, graph: dict, tool: str):
        self.tool = _choice(tool, TOOLS, "tool")
        self.directed = _boolean(graph.get("directed"), "graph directed metadata")
        self.multigraph = _boolean(graph.get("multigraph"), "graph multigraph metadata")
        nodes = _array(graph.get("nodes"), "graph nodes", 1_000_000)
        self.nodes = {}
        self.anchors = defaultdict(list)
        for node in nodes:
            node = _object(node, "graph node")
            identifier = _text(node.get("id"), "node id", 1024)
            if identifier in self.nodes:
                raise ValueError("duplicate graph node identity")
            self.nodes[identifier] = node
            file, line, names = _node_anchor(node, tool)
            for name in names:
                self.anchors[file, line, name].append(identifier)
        self.edges = defaultdict(list)
        for edge in _array(graph.get("edges", graph.get("links", [])), "graph edges", 4_000_000):
            source = edge.get("_src", edge.get("source"))
            target = edge.get("_tgt", edge.get("target"))
            if source not in self.nodes or target not in self.nodes:
                raise ValueError("dangling graph edge")
            relation = _text(edge.get("kind", edge.get("relation")), "edge relation", 128)
            self.edges[source, target, relation].append(edge)

    def resolve(self, identity: Identity) -> list[str]:
        return self.anchors.get((identity.file, identity.start_line, identity.symbol), [])

    def fact_status(self, fact: dict) -> str:
        identities = [Identity.parse(fact["identity"])]
        relation = _relation(fact.get("relation"))
        if relation:
            identities += [relation[0], relation[1]]
        counts = [len(self.resolve(i)) for i in identities]
        if any(n > 1 for n in counts):
            return "ambiguous"
        if any(n == 0 for n in counts):
            return "missing"
        if relation:
            if not self.directed:
                return "unavailable_direction_contract"
            source, target = self.resolve(relation[0])[0], self.resolve(relation[1])[0]
            edges = self.edges.get((source, target, relation[2]), [])
            observed = Counter(_edge_site(e, self.tool) for e in edges)
            expected = Counter((f, n) for f, n, _ in relation[4])
            if expected - observed:
                return "missing"
        return "present"


def _span(raw: dict, output: bytes) -> None:
    raw = _object(raw, "captured output span")
    start = _integer(raw.get("startByte"), "span start", 0, len(output))
    end = _integer(raw.get("endByte"), "span end", start + 1, len(output))
    if sha(output[start:end]) != _digest(raw.get("sha256"), "span SHA-256"):
        raise ValueError("normalized answer span is not in the captured first page")


def score_task(task: dict, observation: dict, judgment: dict, graph: GraphIndex | None = None,
               output: bytes | None = None) -> dict:
    """Score typed, independently reviewed facts, never matching expected prose.

    graph/output are required by the file-backed auditor; None is for isolated
    synthetic scorer-contract tests only and does not establish provenance.
    """
    disposition = _choice(observation.get("disposition"),
                          ("answered", "ambiguous", "abstain", "error", "limit"), "answer disposition")
    ranking_kind = _choice(observation.get("rankingKind"),
                           ("retrieval", "display", "unavailable"), "ranking interpretation")
    ranked = _array(observation.get("ranked"), "ranked candidates", MAX_CANDIDATES)
    identities, page, node_ids = [], [], []
    for candidate in ranked:
        identity = Identity.parse(candidate.get("identity"))
        identities.append(identity)
        node_id = _text(candidate.get("nodeId"), "returned graph node id", 1024)
        node_ids.append(node_id)
        first = _boolean(candidate.get("firstPage"), "first-page candidate")
        if graph is not None and graph.resolve(identity) != [node_id]:
            raise ValueError("returned declaration does not uniquely resolve to its captured graph node")
        if first:
            page.append(identity)
            if output is not None:
                _span(candidate.get("span"), output)
    _unique(identities, "ranked declaration identities")
    _unique(node_ids, "ranked node IDs")
    if ranking_kind == "unavailable" and ranked:
        raise ValueError("unavailable ranking must not invent a ranked list; use display order explicitly")
    relevant = {Identity.parse(x["identity"]): x["grade"] for x in task["relevant"]}
    candidate_reviews = _array(judgment.get("candidates", []), "candidate reviews", MAX_CANDIDATES)
    reviewed = {}
    for item in candidate_reviews:
        identity = Identity.parse(item.get("identity"))
        grade = _integer(item.get("grade"), "reviewed candidate grade", 0, 3)
        if identity in reviewed:
            raise ValueError("duplicate candidate review")
        if identity in relevant and relevant[identity] != grade:
            raise ValueError("post-output review silently changes a frozen relevance judgment")
        reviewed[identity] = grade
    unjudged_candidates = [i for i in page if i not in relevant and i not in reviewed]
    new_positives = [i for i, grade in reviewed.items() if grade >= 2 and i not in relevant]
    grades = {**reviewed, **relevant}
    facts = {x["id"]: x for x in task["facts"]}
    essential = {key for key, fact in facts.items() if fact["essential"]}
    claims = _array(observation.get("claims"), "returned claims", MAX_CLAIMS)
    labels = _array(judgment.get("claims"), "claim judgments", MAX_CLAIMS)
    labels_by_id = {}
    for label in labels:
        identifier = _text(label.get("id"), "reviewed claim id", 128)
        if identifier in labels_by_id:
            raise ValueError("duplicate claim review")
        labels_by_id[identifier] = label
    seen_claims, semantic_claims, supported, unsupported, unjudged, recovered = set(), set(), 0, 0, 0, set()
    for claim in claims:
        identifier = _text(claim.get("id"), "claim id", 128)
        if identifier in seen_claims:
            raise ValueError("duplicate returned claim id")
        seen_claims.add(identifier)
        if output is not None:
            _span(claim.get("span"), output)
        label = labels_by_id.get(identifier)
        identity = Identity.parse(claim.get("identity"))
        relation = _relation(claim.get("relation"), observed=True)
        fact_id = label.get("factId") if label else None
        extra_site = _site(label["source"]) if label and fact_id is None and "source" in label else None
        semantic = (identity, relation, fact_id, extra_site)
        if semantic in semantic_claims:
            raise ValueError("duplicate semantic claims cannot inflate precision")
        semantic_claims.add(semantic)
        verdict = _choice(label.get("verdict"), ("supported", "unsupported", "unjudged"),
                          "claim verdict") if label else "unjudged"
        if label:
            _text(label.get("reason"), "source judgment rationale")
        if verdict == "supported":
            if fact_id is None:
                # Extra true assertions still need independently reviewed source evidence.
                _site(label.get("source"))
            elif fact_id not in facts:
                raise ValueError("review references an unknown frozen source fact")
            elif (identity != Identity.parse(facts[fact_id]["identity"])
                  or not _relation_matches(facts[fact_id], relation)):
                verdict = "unsupported"
            if verdict == "supported":
                supported += 1
                if fact_id is not None:
                    recovered.add(fact_id)
        if verdict == "unsupported":
            unsupported += 1
        elif verdict == "unjudged":
            unjudged += 1
    if set(labels_by_id) - seen_claims:
        raise ValueError("review contains claims absent from normalized output")
    if disposition == "abstain" and (page or claims):
        raise ValueError("an abstention cannot simultaneously assert candidates or facts")
    good_page = {i for i in page if grades.get(i, 0) >= 2}
    expected = task["expected"]
    complete = not unjudged and not unjudged_candidates and not new_positives
    if expected == "answer":
        success = (disposition == "answered" and bool(good_page) and essential <= recovered
                   and not unsupported and complete)
    elif expected == "ambiguous":
        success = disposition == "ambiguous" and len(good_page) >= 2 and not unsupported and complete
    else:
        success = disposition == "abstain" and not claims and not page
    if disposition in ("error", "limit"):
        success = False
    ranks = [n for n, identity in enumerate(identities, 1) if relevant.get(identity, 0) >= 2]
    gains = [(2 ** relevant.get(i, 0) - 1) for i in identities[:10]]
    ideal = sorted((2 ** g - 1 for g in relevant.values()), reverse=True)[:10]
    dcg = sum(g / math.log2(n + 2) for n, g in enumerate(gains))
    idcg = sum(g / math.log2(n + 2) for n, g in enumerate(ideal))
    instrumented = observation.get("admission") is not None
    admitted = observation.get("admission")
    if instrumented:
        admitted = _array(admitted, "instrumented admission node IDs", 65536)
        for identifier in admitted:
            _text(identifier, "admission node id", 1024)
            if graph is not None and identifier not in graph.nodes:
                raise ValueError("admission trace references an absent graph node")
        _unique(admitted, "admission node ids")
    ledger = []
    for fact_id, fact in sorted(facts.items()):
        identity = Identity.parse(fact["identity"])
        graph_status = graph.fact_status(fact) if graph is not None else "unverified_test_fixture"
        node_matches = graph.resolve(identity) if graph is not None else []
        admission = ("admitted" if any(n in admitted for n in node_matches) else "not_admitted") if instrumented else "not_instrumented"
        rank = identities.index(identity) + 1 if identity in identities else None
        if instrumented and rank is not None and not any(n in admitted for n in node_matches):
            raise ValueError("ranked candidate absent from instrumented admission ledger")
        ledger.append(dict(fact=fact_id, source="verified" if graph is not None else "unverified_test_fixture",
                           graph=graph_status, admission=admission, rank=rank,
                           topFive=rank is not None and rank <= 5,
                           firstPage=fact_id in recovered))
    answerable = expected == "answer"
    return dict(task=task["id"], repository=task["repository"], cluster=task["cluster"], kind=task["kind"],
                disposition=disposition, success=success, answerable=answerable,
                successAt5=bool(ranks and ranks[0] <= 5) if ranking_kind == "retrieval" and ideal else None,
                mrrAt10=1 / ranks[0] if ranking_kind == "retrieval" and ranks and ranks[0] <= 10 else
                0.0 if ranking_kind == "retrieval" and ideal else None,
                ndcgAt10=dcg / idcg if ranking_kind == "retrieval" and idcg else None,
                sourceFactRecall=len(recovered) / len(facts) if facts else None,
                supportedClaims=supported, unsupportedClaims=unsupported, unjudgedClaims=unjudged,
                claimPrecision=supported / (supported + unsupported) if supported + unsupported else None,
                precisionWithEmptyZero=supported / (supported + unsupported + unjudged) if claims else 0.0,
                falseEmpty=answerable and (disposition == "abstain" or not page and not claims),
                judgmentComplete=complete, postOutputPositiveCount=len(new_positives),
                unjudgedCandidateCount=len(unjudged_candidates), ledger=ledger)


def _binomial_cdf(k: int, n: int, probability: float) -> float:
    if k < 0:
        return 0.0
    if probability <= 0:
        return 1.0
    if probability >= 1:
        return float(k == n)
    logs = [math.lgamma(n + 1) - math.lgamma(i + 1) - math.lgamma(n - i + 1)
            + i * math.log(probability) + (n - i) * math.log1p(-probability)
            for i in range(k + 1)]
    largest = max(logs)
    return min(1.0, math.exp(largest) * math.fsum(math.exp(x - largest) for x in logs))


def _binomial_upper(successes: int, total: int, alpha: float = .025) -> float:
    """Exact one-sided Clopper-Pearson bound, including zero observed errors."""
    if not total or successes == total:
        return 1.0
    if successes == 0:
        return -math.expm1(math.log(alpha) / total)
    low, high = successes / total, 1.0
    for _ in range(60):
        middle = (low + high) / 2
        if _binomial_cdf(successes, total, middle) > alpha:
            low = middle
        else:
            high = middle
    return high


def finite_ni_power(total: int, adverse_probability: float, margin: float = .02) -> dict:
    """Prospective lower bound on feasibility under the registered no-win model.

    This is the power of the finite bound only, not of the additional
    hierarchical bootstrap gate. The probability is an assumption fixed before
    collection; it must never be estimated from the held-out answers.
    """
    _integer(total, "independent control clusters", 0, MAX_TASKS)
    _number(adverse_probability, "planned adverse-cluster probability", 0, 1)
    _number(margin, "NI margin", .0001, .1)
    low, high = -1, total
    while high - low > 1:
        middle = (low + high) // 2
        if _binomial_upper(middle, total) < margin:
            low = middle
        else:
            high = middle
    probability = _binomial_cdf(low, total, adverse_probability) if low >= 0 else 0.0
    return dict(independentClusters=total, maximumAdverseClusters=low,
                assumedAdverseProbability=adverse_probability, finiteBoundPassProbability=probability,
                method="exact_binomial_no_definite_wins_planning/1",
                scope="Conditional fixed panel, independent source-task clusters; hierarchical bootstrap may reduce power.")


def paired_interval(rows: list[tuple[str, str, float, float]], *, samples: int = 2000,
                    seed: int = 29) -> dict:
    """Deterministic paired hierarchical bootstrap: repository then source task.

    Paraphrases share a cluster and are averaged before resampling. Equal
    repository weight is intentional. An exact finite-task risk bound prevents
    all-tie bootstraps from inventing perfect knowledge of unseen discordance.
    """
    _integer(samples, "bootstrap resamples", 1000, 10000)
    grouped = defaultdict(lambda: defaultdict(list))
    for repo, cluster, compass, other in rows:
        a = _number(compass, "paired score", 0, 1)
        b = _number(other, "paired score", 0, 1)
        grouped[repo][cluster].append(a - b)
    groups = [[mean(values) for _, values in sorted(clusters.items())]
              for _, clusters in sorted(grouped.items())]
    if not groups:
        return dict(estimate=None, lower=None, upper=None, repositories=0, clusters=0,
                    method="paired_repository_then_task_cluster_bootstrap/1", degenerate=True)
    estimate = mean(mean(values) for values in groups)
    rng = random.Random(seed)
    boot = []
    for _ in range(samples):
        selected = [groups[rng.randrange(len(groups))] for _ in groups]
        boot.append(mean(mean(values[rng.randrange(len(values))] for _ in values) for values in selected))
    boot.sort()
    # Conditional on the fixed repository panel and independent source-task
    # sampling, a task's score difference is at least -1 if ANY paraphrase
    # loses and at least +1 only if EVERY paraphrase wins by one. Ignoring
    # intermediate gains is conservative. Bonferroni .025 bounds give a 95%
    # lower bound on P(definite win)-P(any loss), including zero-loss samples.
    cluster_values = [values for clusters in grouped.values() for values in clusters.values()]
    count = len(cluster_values)
    win = sum(all(v == 1 for v in values) for values in cluster_values)
    loss = sum(any(v < 0 for v in values) for values in cluster_values)
    definite_loss = sum(all(v == -1 for v in values) for values in cluster_values)
    any_win = sum(any(v > 0 for v in values) for values in cluster_values)
    balanced = len({len(g) for g in groups}) == 1
    finite_lower = 1 - _binomial_upper(count - win, count) - _binomial_upper(loss, count) if balanced else -1.0
    finite_upper = _binomial_upper(any_win, count) - (1 - _binomial_upper(count - definite_loss, count)) if balanced else 1.0
    bootstrap_lower, bootstrap_upper = boot[int(.025 * samples)], boot[int(.975 * samples) - 1]
    return dict(estimate=estimate, lower=max(-1.0, min(bootstrap_lower, finite_lower)),
                upper=min(1.0, max(bootstrap_upper, finite_upper)),
                bootstrapLower=bootstrap_lower, bootstrapUpper=bootstrap_upper,
                finiteTaskRiskLower=finite_lower, finiteTaskRiskUpper=finite_upper,
                finiteRiskMethod="exact_paired_task_cluster_risk_bonferroni/1",
                finiteRiskScope="Independent source-task clusters conditional on this fixed repository panel; not population repository inference.",
                balancedIndependentClusters=balanced,
                repositories=len(groups), clusters=sum(map(len, groups)),
                method="paired_repository_then_task_cluster_bootstrap/1", seed=seed, resamples=samples,
                degenerate=max(boot) - min(boot) < 1e-12)


def _summary(rows: list[dict]) -> dict:
    def avg(key, selected=None):
        vals = [r[key] for r in (rows if selected is None else selected) if r[key] is not None]
        return mean(vals) if vals else None
    answerable = [r for r in rows if r["answerable"]]
    total_claims = sum(r["supportedClaims"] + r["unsupportedClaims"] for r in rows)
    return dict(tasks=len(rows), firstPageSuccess=avg("success"), answerableSuccess=avg("success", answerable),
                successAt5=avg("successAt5"), mrrAt10=avg("mrrAt10"), ndcgAt10=avg("ndcgAt10"),
                sourceFactRecall=avg("sourceFactRecall"), falseEmptyRate=avg("falseEmpty", answerable),
                claimPrecision=sum(r["supportedClaims"] for r in rows) / total_claims if total_claims else None,
                unsupportedClaimRate=sum(r["unsupportedClaims"] for r in rows) / total_claims if total_claims else None,
                unjudgedClaims=sum(r["unjudgedClaims"] for r in rows),
                unjudgedCandidates=sum(r["unjudgedCandidateCount"] for r in rows),
                byKind={k:dict(tasks=len(v), success=mean(r["success"] for r in v))
                        for k in KINDS if (v := [r for r in rows if r["kind"] == k])})


def _cost_summary(rows: list[dict]) -> dict:
    """All executions, including failures; p95 uses the nearest-rank definition."""
    def quantile(key, percentile):
        values = sorted(r[key] for r in rows)
        return values[max(0, math.ceil(len(values) * percentile) - 1)] if values else None
    memory = [r["peakRssBytes"] for r in rows if r["peakRssBytes"] is not None]
    return dict(executions=len(rows), failures=sum(r["failed"] for r in rows),
                overBudget=sum(r["overBudget"] for r in rows),
                firstPageBytes=sum(r["firstPageBytes"] for r in rows),
                workflowBytes=sum(r["workflowBytes"] for r in rows),
                followUps=sum(r["followUps"] for r in rows),
                p50WorkflowWallMs=quantile("workflowWallMs", .5),
                p95WorkflowWallMs=quantile("workflowWallMs", .95),
                maximumPeakRssBytes=max(memory) if memory else None,
                memoryMeasurements=len(memory),
                conditions=sorted({r["condition"] for r in rows}))


class Artifacts:
    def __init__(self, root: Path):
        self.root = root.resolve()
        self.seen = {}
        self.total = 0

    def read(self, ref: dict, maximum: int = MAX_JSON_BYTES) -> bytes:
        ref = _object(ref, "artifact reference")
        relative = _relative(ref.get("path"))
        path = (self.root / relative).resolve()
        if not path.is_relative_to(self.root):
            raise ValueError("artifact escapes evidence root")
        expected = _digest(ref.get("sha256"))
        data = read_bounded(path, maximum)
        if sha(data) != expected:
            raise ValueError(f"artifact digest mismatch: {relative}")
        previous = self.seen.get(relative)
        if previous is not None and previous != expected:
            raise ValueError("conflicting artifact identity")
        if previous is None:
            self.total += len(data)
            if self.total > 2 * 1024**3 or len(self.seen) >= 65536:
                raise ValueError("aggregate artifact read limit exceeded")
        self.seen[relative] = expected
        return data

    def document(self, ref: dict, maximum: int = MAX_JSON_BYTES) -> dict:
        return _json(self.read(ref, maximum))

    def recheck(self) -> None:
        for path, digest in sorted(self.seen.items()):
            if sha(read_bounded(self.root / path, MAX_GRAPH_BYTES)) != digest:
                raise ValueError("artifact changed during audit")


def _verify_sources(registration: dict, roots: dict) -> dict[str, dict[str, bytes]]:
    result = {}
    total = 0
    for repo in registration["repositories"]:
        root = Path(_text(roots.get(repo["id"]), "local source root", 4096)).resolve()
        files = {}
        for entry in repo["files"]:
            path = (root / entry["file"]).resolve()
            if not path.is_relative_to(root):
                raise ValueError("source file escapes registered root")
            data = read_bounded(path, MAX_SOURCE_BYTES)
            total += len(data)
            if total > 256 * 1024**2:
                raise ValueError("aggregate source file read limit exceeded")
            if sha(data) != entry["sha256"]:
                raise ValueError("source file digest mismatch")
            files[entry["file"]] = data
        result[repo["id"]] = files
    for task in registration["tasks"]:
        identities = [Identity.parse(item["identity"]) for item in task["relevant"]]
        for fact in task["facts"]:
            sites = [_site(fact["source"])]
            relation = _relation(fact.get("relation"))
            if relation:
                _boolean(fact.get("requireOccurrences", True), "required answer occurrences")
                sites.extend(relation[4])
                identities.extend(relation[:2])
            for file, line, text in sites:
                lines = result[task["repository"]][file].decode("utf-8").splitlines()
                if line > len(lines) or lines[line - 1].strip() != text.strip():
                    raise ValueError("source witness does not match the pinned source line")
        for identity in identities:
            if identity.start_line > len(result[task["repository"]][identity.file].splitlines()):
                raise ValueError("declaration anchor is outside its pinned source file")
    return result


def verify_build_capture(capture: dict, read) -> bool:
    """Check raw bounded build evidence; success flags alone are insufficient.

    ``read`` resolves either external build inputs or contained artifact refs.
    This checks recorded integrity, not whether an attested subprocess ran.
    """
    capture = _object(capture, "bounded build capture")
    for arg in _array(capture.get("argv"), "build argv", 128, 1):
        _text(arg, "build argv entry", 4096)
    for stream in ("stdout", "stderr"):
        data = read(capture.get(stream))
        if _integer(capture.get(stream + "_bytes"), "build stream bytes") != len(data):
            raise ValueError("build byte counts disagree with immutable raw output")
    _integer(capture.get("wall_ms"), "build latency", 0, 86_400_000)
    timed_out = _boolean(capture.get("timed_out"), "build timeout")
    limited = _boolean(capture.get("output_limited"), "build output-limit status")
    code = _integer(capture.get("exit_code"), "build exit status", -255, 255)
    return code == 0 and not timed_out and not limited


def _review_failures(review: dict, where: str) -> list[str]:
    """Attestations are explicit inputs, never inferred from a pass boolean."""
    review = review if isinstance(review, dict) else {}
    failures = []
    if review.get("status") != "confirmed":
        failures.append(f"{where}: independent review is pending")
    if review.get("reviewerType") not in ("human", "ai") or not review.get("reviewer"):
        failures.append(f"{where}: independent reviewer identity/type not attested")
    if review.get("reviewer") is not None:
        _text(review["reviewer"], "independent reviewer identifier", 256)
    if review.get("toolLabelsBlinded") is not True:
        failures.append(f"{where}: tool-blinded review not attested")
    if review.get("independentOfAuthor") is not True or review.get("sourceAware") is not True:
        failures.append(f"{where}: independent source-aware review not attested")
    return failures


def _repository_key(value: Any) -> str:
    """Canonical HTTPS origin, without guessing renamed/forked repositories."""
    value = _text(value, "exposure source URL", 2048)
    parsed = urlsplit(value)
    if (parsed.scheme != "https" or not parsed.hostname or parsed.username is not None
            or parsed.password is not None or parsed.port not in (None, 443)
            or parsed.query or parsed.fragment or "%" in parsed.path or "\\" in value):
        raise ValueError("exposure source URL requires an unambiguous HTTPS origin")
    host = parsed.hostname.lower().rstrip(".")
    if not re.fullmatch(r"[a-z0-9.-]+", host):
        raise ValueError("invalid exposure source host")
    if host == "www.github.com":
        host = "github.com"
    path = parsed.path.rstrip("/")
    if host == "github.com":
        path = path.lower()
    if path.endswith(".git"):
        path = path[:-4]
    if not path or any(part in ("", ".", "..") for part in path.split("/")[1:]):
        raise ValueError("invalid exposure repository path")
    if host == "github.com" and len(path.split("/")) != 3:
        raise ValueError("exposure URL requires a GitHub repository root")
    return host + path


def exposure_failures(registration: dict, read_evidence=None) -> list[str]:
    """Check the recorded lineage, not the truth/completeness of its attestation.

    Missing lineage keeps old diagnostic inputs readable but ineligible. A
    supplied reader verifies immutable historical evidence before any capture.
    No question text, result, current task ID or commit can erase source exposure.
    """
    lineage = registration.get("exposureLineage")
    if lineage is None:
        return ["frozen source/mechanism exposure lineage is missing"]
    lineage = _object(lineage, "exposure lineage")
    if lineage.get("schema") != "compass.broad-query-exposure-lineage/1":
        raise ValueError("unsupported exposure lineage schema")
    frozen = _timestamp(registration.get("frozenAt"))
    if (_timestamp(lineage.get("frozenAt")) != frozen
            or _timestamp(lineage.get("historyCompleteThrough")) != frozen):
        raise ValueError("exposure lineage and history cutoff must match source freeze")
    author = _text(lineage.get("author"), "exposure history author", 256)
    failures = []
    if not _boolean(lineage.get("historyComplete"), "exposure history completeness"):
        failures.append("exposure lineage history is explicitly incomplete")
    review = _object(lineage.get("independentReview", {}), "exposure history review")
    if (review.get("status") != "confirmed" or review.get("reviewerType") not in ("human", "ai")
            or review.get("sourceAware") is not True or review.get("independentOfAuthor") is not True):
        failures.append("exposure lineage independent history review is pending")
    reviewer = review.get("reviewer")
    if reviewer is not None:
        _text(reviewer, "exposure history reviewer", 256)
    if not reviewer or reviewer in (author, registration.get("sourceReview", {}).get("author")):
        failures.append("exposure lineage requires a distinct independent reviewer")
    reviewed = _timestamp(review["reviewedAt"]) if review.get("status") == "confirmed" else frozen
    if reviewed > frozen:
        raise ValueError("exposure history review follows source freeze")

    # Bounded equivalence classes retain explicit cross-repository mechanism
    # links and repository renames. Different wording never creates a new unit.
    parents = {}

    def root(key):
        if key not in parents:
            if len(parents) >= 32768:
                raise ValueError("exposure lineage identity limit exceeded")
            parents[key] = key
        while parents[key] != key:
            parents[key] = parents[parents[key]]
            key = parents[key]
        return key

    def mechanism(value):
        value = _text(value, "stable mechanism ID", 128)
        if not re.fullmatch(r"[a-z0-9][a-z0-9._:/-]*", value):
            raise ValueError("mechanism IDs require stable lowercase ASCII spelling")
        return "mechanism:" + value

    for field, key in (("repositoryAliases", lambda v: "repository:" + _repository_key(v)),
                       ("mechanismLinks", mechanism)):
        for pair in _array(lineage.get(field), field, 8192):
            pair = _array(pair, field + " pair", 2, 2)
            a, b = (root(key(value)) for value in pair)
            if a != b:
                parents[max(a, b)] = min(a, b)

    history = _array(lineage.get("history"), "exposure history", 8192)
    seen, digests, exposed_repos, exposed_mechanisms = set(), set(), set(), set()
    exposed_commits = set()
    evidence_bytes = 0
    for event in history:
        event = _object(event, "exposure event")
        identifier = _text(event.get("id"), "exposure event ID", 128)
        if identifier in seen:
            raise ValueError("duplicate exposure event ID")
        seen.add(identifier)
        if _timestamp(event.get("observedAt")) > reviewed:
            raise ValueError("exposure event follows independent history review")
        _choice(event.get("kind"), ("source_development", "graph_build", "query_capture",
                                   "answer_review", "implementation_tuning"), "exposure kind")
        if not re.fullmatch(r"(?:[0-9a-f]{40}|[0-9a-f]{64})", str(event.get("commit", ""))):
            raise ValueError("exposure commit requires a full hexadecimal object id")
        exposed_commits.add(event["commit"])
        exposed_repos.add(root("repository:" + _repository_key(event.get("repositoryUrl"))))
        for value in _array(event.get("mechanisms"), "exposed mechanisms", 32):
            exposed_mechanisms.add(root(mechanism(value)))
        digest = _digest(event.get("evidenceSha256"), "exposure evidence SHA-256")
        if digest not in digests and read_evidence is not None:
            data = read_evidence(digest)
            if not isinstance(data, bytes) or len(data) > MAX_JSON_BYTES or sha(data) != digest:
                raise ValueError("exposure evidence bytes differ from frozen history")
            evidence_bytes += len(data)
            if evidence_bytes > 512 * 1024**2:
                raise ValueError("exposure history evidence exceeds aggregate byte limit")
        digests.add(digest)
    if digests and read_evidence is None:
        failures.append("exposure lineage historical evidence bytes were not verified")

    repo_roots = {}
    commits = {}
    for repo in registration["repositories"]:
        if not re.fullmatch(r"(?:[0-9a-f]{40}|[0-9a-f]{64})", str(repo.get("commit", ""))):
            raise ValueError("exposure source requires a full hexadecimal commit")
        key = root("repository:" + _repository_key(repo["url"]))
        if key in repo_roots.values():
            raise ValueError("exposure split repeats an aliased repository")
        if repo["commit"] in commits.values():
            raise ValueError("exposure split repeats a source commit")
        repo_roots[repo["id"]] = key
        commits[repo["id"]] = repo["commit"]
    for task in registration["tasks"]:
        split = _choice(task.get("split"), ("development", "confirmation"), "task exposure split")
        keys = [mechanism(value) for value in
                _array(task.get("mechanisms"), "task mechanism lineage", 32, 1)]
        _unique(keys, "task mechanism IDs")
        keys = {root(value) for value in keys}
        if registration["designation"] == "heldout" and split != "confirmation":
            failures.append(f"{task['id']}: heldout registration contains a development task")
        if split == "confirmation" and (repo_roots[task["repository"]] in exposed_repos
                                         or commits[task["repository"]] in exposed_commits
                                         or keys & exposed_mechanisms):
            failures.append(f"{task['id']}: confirmation source or mechanism was previously exposed")
    return failures


def _policy(registration: dict) -> dict:
    policy = _object(registration.get("gatePolicy", {}), "gate policy")
    # The registered implementation cannot weaken the plan's proposed gates.
    return dict(minimumLift=_number(policy.get("minimumLift", .10), "minimum lift", .10, 1),
                noninferiorityMargin=_number(policy.get("noninferiorityMargin", .02), "NI margin", 0, .02),
                minimumRepositories=_integer(policy.get("minimumRepositories", 8), "minimum repositories", 8, 32),
                minimumBroadClusters=_integer(policy.get("minimumBroadClusters", 80), "minimum broad clusters", 80, MAX_TASKS),
                resamples=_integer(policy.get("resamples", 2000), "bootstrap resamples", 1000, 10000),
                seed=_integer(policy.get("seed", 29), "bootstrap seed", 0, 2**32 - 1))


def assess(manifest: dict, root: Path) -> dict:
    """Recompute a report from frozen registration, raw captures, and judgments."""
    if manifest.get("schema") != MANIFEST_SCHEMA:
        raise ValueError("unsupported broad-query review-input schema")
    artifacts = Artifacts(root)
    registration = validate_registration(artifacts.document(manifest.get("registration")))
    run = artifacts.document(manifest.get("run"))
    reviews = artifacts.document(manifest.get("reviews"))
    if run.get("schema") != RUN_SCHEMA or reviews.get("schema") != REVIEW_SCHEMA:
        raise ValueError("unsupported capture or judgments schema")
    if run.get("registrationSha256") != manifest["registration"]["sha256"]:
        raise ValueError("run is not bound to the frozen source registration")
    if reviews.get("runSha256") != manifest["run"]["sha256"]:
        raise ValueError("judgments are not bound to the captured run")
    started = _timestamp(run.get("startedAt"))
    if _timestamp(registration["frozenAt"]) >= started:
        raise ValueError("source registration was not frozen before collection")
    sources = _verify_sources(registration, _object(manifest.get("sourceRoots"), "source roots"))
    evidence_refs = _object(manifest.get("exposureEvidence", {}), "exposure evidence references")
    expected_exposure = {event["evidenceSha256"] for event in
                         registration.get("exposureLineage", {}).get("history", [])}
    if set(evidence_refs) != expected_exposure:
        raise ValueError("exposure evidence references differ from frozen history")
    failures = exposure_failures(registration, lambda digest: artifacts.read(evidence_refs.get(digest)))
    if registration["designation"] != "heldout" or registration.get("qualificationEligible") is not True:
        failures.append("registration is development or explicitly ineligible")
    failures += _review_failures(registration.get("independentReview"), "frozen source oracles")
    if registration.get("independentReview", {}).get("reviewer") == registration.get("sourceReview", {}).get("author"):
        failures.append("source author and independent source reviewer must be distinct")
    if registration.get("sourceReview", {}).get("blindToOutputs") is not True:
        failures.append("source-first authoring before outputs is not attested")
    policy = _policy(registration)
    repositories = {r["id"]: r for r in registration["repositories"]}
    tasks = {t["id"]: t for t in registration["tasks"]}
    if len(repositories) < policy["minimumRepositories"]:
        failures.append("insufficient independent repositories")
    languages = Counter(r["language"] for r in repositories.values())
    if len(languages) < 4 or any(n < 2 for n in languages.values()):
        failures.append("four languages with at least two repositories each required")
    cohorts = registration.get("metricCohorts")
    if cohorts is None:
        distribution = {"broad": 10, "exact_or_directed": 4, "multihop": 2, "ambiguous": 2, "unsupported": 2}
        ratios = []
        for repo in repositories:
            counts = Counter(t["kind"] for t in tasks.values() if t["repository"] == repo)
            counts["exact_or_directed"] = counts["exact"] + counts["directed"]
            ratio = counts["broad"] / 10
            ratios.append(ratio)
            if ratio < 1 or any(counts[k] != expected * ratio for k, expected in distribution.items()):
                failures.append(f"{repo}: task-kind expansion is not balanced 10:4:2:2:2")
        if len(set(ratios)) != 1:
            failures.append("repository task counts are unbalanced")
    else:
        # Heterogeneous prospective panels retain every captured task. Only the
        # source-frozen cohort IDs select primary comparisons; all other tasks
        # remain visible in diagnostics. No output-based task selection exists.
        for metric, cohort in cohorts.items():
            selected = [tasks[t] for t in cohort["taskIds"]]
            strata = Counter(repositories[r]["language"] for r in {t["repository"] for t in selected})
            if len(strata) < 4 or len(set(strata.values())) != 1:
                failures.append(f"{metric}: frozen cohort language allocation is not balanced")
    frozen_capture_plan = registration.get("capturePlan")
    if not isinstance(frozen_capture_plan, dict):
        failures.append("actual command/binary/normalizer capture plan was not frozen before collection")
    elif frozen_capture_plan.get("assessorSha256") != sha(read_bounded(Path(__file__), MAX_JSON_BYTES)):
        failures.append("assessor differs from the source-frozen analysis protocol")
    if isinstance(frozen_capture_plan, dict):
        from benchmarks.agent_query.broad_query_capture import child_environment
        if run.get("childEnvironment") != child_environment(frozen_capture_plan, artifacts.root):
            raise ValueError("captured environment differs from source-frozen isolation policy")
        for field, frozen_ref in (("measurementExecutable", frozen_capture_plan["measurement"]["executable"]),
                                  ("environmentExecutable", frozen_capture_plan["environmentExecutable"])):
            artifacts.read(run.get(field), MAX_GRAPH_BYTES)
            if run[field]["sha256"] != frozen_ref["sha256"]:
                raise ValueError("measurement/environment launcher differs from source-frozen binary")
    tools = _array(run.get("tools"), "tool identities", 2, 2)
    tool_map = {}
    for tool in tools:
        name = _choice(tool.get("id"), TOOLS, "tool")
        if name in tool_map:
            raise ValueError("duplicate tool identity")
        tool_map[name] = tool
        _text(tool.get("version"), "tool version", 128)
        _text(tool.get("commit"), "tool source commit", 128)
        artifacts.read(tool.get("executable"), MAX_GRAPH_BYTES)
        artifacts.read(tool.get("environment"))
        artifacts.read(tool.get("normalizer"))
        if isinstance(frozen_capture_plan, dict):
            frozen_tools = _array(frozen_capture_plan.get("tools"), "source-frozen tools", 2, 2)
            frozen = [t for t in frozen_tools if t.get("id") == name]
            if len(frozen) != 1:
                raise ValueError("source-frozen tool identities are incomplete or duplicated")
            frozen = frozen[0]
            for field in ("version", "commit", "nativeOffline", "settings"):
                if tool.get(field) != frozen.get(field):
                    raise ValueError("actual tool configuration differs from source-frozen settings")
            for field in ("executable", "environment"):
                if tool.get(field, {}).get("sha256") != frozen.get(field, {}).get("sha256"):
                    raise ValueError("actual tool/dependency bytes differ from source-frozen identity")
            if tool["normalizer"]["sha256"] != frozen_capture_plan.get("normalizerSha256"):
                raise ValueError("actual normalizer differs from source-frozen protocol")
        if tool.get("nativeOffline") is not True:
            failures.append(f"{name}: native offline mode not declared")
        settings = _object(tool.get("settings"), "recorded extraction/query settings")
        for field in ("scopePolicy", "inferenceProfile", "clustering", "queryInterface"):
            if not isinstance(settings.get(field), str) or not settings[field]:
                failures.append(f"{name}: recorded {field} setting is missing")
            else:
                _text(settings[field], f"recorded {field}", 4096)
    if set(tool_map) != set(TOOLS):
        raise ValueError("both tool identities are required")
    competitor = tool_map["graphify"]
    release = competitor.get("releaseVerification")
    if not isinstance(release, dict):
        failures.append("latest Graphify release verification missing")
    else:
        release_doc = artifacts.document(release)
        if (release_doc.get("version"), release_doc.get("commit")) != (competitor["version"], competitor["commit"]):
            raise ValueError("release verification conflicts with comparator identity")
        if _timestamp(release_doc.get("checkedAt")) > started:
            raise ValueError("comparator release selected after collection")
        if release_doc.get("source") != "https://github.com/Graphify-Labs/graphify/releases":
            failures.append("release verification is not attributed to the official release tracker")
    graph_indexes = {}
    inventory_hashes = defaultdict(set)
    graph_records = _array(run.get("graphs"), "captured graphs", 64)
    for entry in graph_records:
        key = (entry.get("repository"), entry.get("tool"))
        if key in graph_indexes or key[0] not in repositories or key[1] not in TOOLS:
            raise ValueError("duplicate or unexpected captured graph")
        graph_indexes[key] = GraphIndex(artifacts.document(entry.get("artifact"), MAX_GRAPH_BYTES), key[1])
        if entry.get("sourceCommit") != repositories[key[0]]["commit"]:
            raise ValueError("graph source pin conflicts with source registration")
        if entry.get("sourceCleanBeforeAndAfter") is not True:
            failures.append(f"{key}: source cleanliness not captured")
        inventory = artifacts.document(entry.get("sourceInventory"))
        if inventory.get("files") != repositories[key[0]]["files"]:
            raise ValueError("graph build source inventory differs from source registration")
        inventory_hashes[key[0]].add(entry["sourceInventory"]["sha256"])
        build = artifacts.document(entry.get("buildCapture"))
        if not verify_build_capture(build, artifacts.read):
            failures.append(f"{key}: clean successful bounded build capture is incomplete")
        if isinstance(frozen_capture_plan, dict):
            original = artifacts.document(entry.get("originalBuildCapture"))
            frozen_graph = next(g for g in frozen_capture_plan["graphs"]
                                if (g["repository"], g["tool"]) == key)
            if entry["originalBuildCapture"]["sha256"] != frozen_graph["buildCapture"]["sha256"]:
                raise ValueError("build evidence differs from source-frozen graph input")
            if any(entry[field]["sha256"] != frozen_graph[field]["sha256"]
                   for field in ("artifact", "sourceInventory")):
                raise ValueError("graph bytes/inventory differ from source-frozen graph input")
            if ({k: v for k, v in build.items() if k not in ("stdout", "stderr")} !=
                    {k: v for k, v in original.items() if k not in ("stdout", "stderr")} or
                    any(build[stream]["sha256"] != original[stream]["sha256"]
                        for stream in ("stdout", "stderr"))):
                raise ValueError("rebased build evidence changed original capture fields")
    if set(graph_indexes) != {(repo, tool) for repo in repositories for tool in TOOLS}:
        raise ValueError("missing captured graphs")
    if any(len(values) != 1 for values in inventory_hashes.values()):
        failures.append("paired tools did not receive an identical declared source inventory")
    arms = _array(run.get("arms"), "cost arms", 2, 2)
    arm_map = {a.get("id"): a for a in arms}
    if len(arm_map) != 2 or set(arm_map) != set(ARMS):
        raise ValueError("default and normalized cost arms both required")
    budget = _integer(arm_map["normalized"].get("budgetBytes"), "normalized UTF-8 byte budget", 128, 1024**2)
    if arm_map["normalized"].get("measure") != "utf8_bytes" or arm_map["normalized"].get("overflow") != "fail":
        raise ValueError("v1 normalized arm requires a frozen UTF-8 byte budget and overflow=fail")
    # Arm choice must be frozen with the source registration, not after observing results.
    if registration.get("arms") != arms:
        failures.append("cost arms are not frozen identically in the registration")
    review_rows = _array(reviews.get("observations"), "review rows", MAX_TASKS * 4)
    review_map = {}
    for row in review_rows:
        key = (row.get("task"), row.get("tool"), row.get("arm"))
        if key in review_map:
            raise ValueError("duplicate observation review")
        review_map[key] = row
    observations = _array(run.get("observations"), "captured observations", MAX_TASKS * 4)
    scored, costs, observed_keys = {}, [], set()
    for obs in observations:
        key = (obs.get("task"), obs.get("tool"), obs.get("arm"))
        if key in observed_keys or key[0] not in tasks or key[1] not in TOOLS or key[2] not in ARMS:
            raise ValueError("duplicate or unexpected observation")
        observed_keys.add(key)
        if key not in review_map:
            raise ValueError("missing typed observation review")
        judged = review_map[key]
        if judged.get("normalizedSha256") != obs.get("normalized", {}).get("sha256"):
            raise ValueError("review not bound to normalized response")
        failures += _review_failures(judged.get("independentReview"), f"{key}: output normalization/facts")
        normalized = artifacts.document(obs.get("normalized"))
        if normalized.get("normalizationComplete") is not True:
            failures.append(f"{key}: output normalization remains explicitly incomplete")
        if normalized.get("schema") != NORMALIZED_SCHEMA:
            raise ValueError("unsupported normalized-answer schema")
        if any(normalized.get(field) != expected for field, expected in zip(("task", "tool", "arm"), key)):
            raise ValueError("normalized response belongs to a different task/tool/arm")
        captures = _array(obs.get("captures"), "bounded runner captures", 4, 1)
        stdout_pages, elapsed, total_bytes, timed_out, limited, bad_exit = [], 0, 0, False, False, False
        for ref in captures:
            capture = artifacts.document(ref)
            argv = _array(capture.get("argv"), "recorded argv", 128, 1)
            for arg in argv:
                _text(arg, "argv entry", 4096)
            if tasks[key[0]]["kind"] in ("broad", "unsupported") and tasks[key[0]]["question"] not in argv:
                raise ValueError("broad/unsupported task question was rewritten or omitted in the measured command")
            if isinstance(frozen_capture_plan, dict):
                from benchmarks.agent_query.broad_query_capture import command, launch
                frozen_tool = next(t for t in frozen_capture_plan["tools"] if t["id"] == key[1])
                frozen_graph = next(g for g in graph_records if g["repository"] == tasks[key[0]]["repository"] and g["tool"] == key[1])
                expected_argv = launch(frozen_capture_plan,
                    command(frozen_capture_plan, frozen_tool, key[2], tasks[key[0]]["question"],
                            artifacts.root / frozen_graph["artifact"]["path"]), artifacts.root)
                if tuple(argv) != expected_argv:
                    raise ValueError("captured argv differs from exact predeclared native command")
            out = artifacts.read(capture.get("stdout"))
            err = artifacts.read(capture.get("stderr"))
            if _integer(capture.get("stdout_bytes"), "stdout bytes") != len(out) or _integer(capture.get("stderr_bytes"), "stderr bytes") != len(err):
                raise ValueError("capture byte counts disagree with immutable raw output")
            elapsed += _integer(capture.get("wall_ms"), "capture latency", 0, 86_400_000)
            total_bytes += len(out)
            stdout_pages.append(out)
            timed_out |= _boolean(capture.get("timed_out"), "capture timeout")
            limited |= _boolean(capture.get("output_limited"), "capture output-limit status")
            bad_exit |= _integer(capture.get("exit_code"), "exit status", -255, 255) != 0
        if key[2] == "normalized" and len(captures) != 1:
            raise ValueError("normalized primary arm permits no follow-ups")
        overflow = key[2] == "normalized" and len(stdout_pages[0]) > budget
        row = score_task(tasks[key[0]], normalized, judged, graph_indexes[tasks[key[0]]["repository"], key[1]], stdout_pages[0])
        # Failed/over-budget executions stay in every denominator, never disappear.
        execution_failed = timed_out or limited or bad_exit or overflow
        if execution_failed:
            row["success"] = False
            row["executionFailed"] = True
        else:
            row["executionFailed"] = False
        if not row["judgmentComplete"]:
            failures.append(f"{key}: unjudged output or post-output source-oracle correction")
        for label in judged.get("claims", []):
            if label.get("verdict") == "supported" and label.get("factId") is None:
                file, line, text = _site(label.get("source"))
                data = sources[tasks[key[0]]["repository"]].get(file)
                lines = data.decode().splitlines() if data is not None else []
                if line > len(lines) or lines[line - 1].strip() != text.strip():
                    raise ValueError("extra supported claim lacks a pinned source witness")
        scored[key] = row
        peak = obs.get("peakRssBytes")
        if peak is not None:
            _integer(peak, "measured peak RSS bytes", 1)
        condition = _choice(obs.get("condition"), ("fresh_cli", "warm_cli", "warm_mcp", "cold_cli", "unrecorded"), "timing condition")
        if condition == "unrecorded" or peak is None:
            failures.append(f"{key}: cost condition or peak RSS measurement unavailable")
        costs.append(dict(task=key[0], tool=key[1], arm=key[2], firstPageBytes=len(stdout_pages[0]),
                          workflowBytes=total_bytes, workflowWallMs=elapsed, followUps=len(captures) - 1,
                          overBudget=overflow, failed=execution_failed,
                          peakRssBytes=peak, condition=condition))
    expected_keys = {(task, tool, arm) for task in tasks for tool in TOOLS for arm in ARMS}
    if observed_keys != expected_keys or set(review_map) != expected_keys:
        raise ValueError("incomplete paired task/tool/arm matrix")
    comparisons = {}
    for name, select, metric in [
        ("broadSuccess", lambda r: r["kind"] == "broad", "success"),
        ("exactDirected", lambda r: r["kind"] in ("exact", "directed"), "success"),
        ("claimPrecisionEmptyZero", lambda r: r["answerable"], "precisionWithEmptyZero"),
    ]:
        pairs = []
        for task_id in sorted(tasks):
            a, b = scored[task_id, "compass", "normalized"], scored[task_id, "graphify", "normalized"]
            if (task_id in cohorts[name]["taskIds"] if cohorts else select(a)):
                pairs.append((a["repository"], a["cluster"], float(a[metric]), float(b[metric])))
        interval = paired_interval(pairs, samples=policy["resamples"], seed=policy["seed"])
        comparisons[name] = interval
        minimum_repos = cohorts[name]["minimumRepositories"] if cohorts else policy["minimumRepositories"]
        minimum_clusters = cohorts[name]["minimumIndependentClusters"] if cohorts else 0
        if (interval["repositories"] < minimum_repos or interval["clusters"] < minimum_clusters
                or not interval.get("balancedIndependentClusters", False)):
            failures.append(f"{name}: insufficient or unbalanced independent comparison clusters")
    primary = comparisons["broadSuccess"]
    if primary["clusters"] < policy["minimumBroadClusters"]:
        failures.append("primary comparison has insufficient independent broad tasks")
    if primary["estimate"] is None or primary["estimate"] < policy["minimumLift"] or primary["lower"] <= 0:
        failures.append("broad-query superiority gate not established")
    for name in ("exactDirected", "claimPrecisionEmptyZero"):
        lower = comparisons[name]["lower"]
        if lower is None or lower <= -policy["noninferiorityMargin"]:
            failures.append(f"{name}: noninferiority not established")
    # Power assumptions must be frozen; these are approximate paired-binary
    # planning calculations, not an estimated post-hoc achieved power.
    power = registration.get("powerAnalysis")
    if not isinstance(power, dict):
        failures.append("prospective power analysis is missing; sample size alone cannot establish power")
    else:
        target = _number(power.get("targetPower"), "planned power", .8, .99)
        effect = policy["minimumLift"]
        discordance = _number(power.get("expectedBroadDiscordance"), "development broad discordance", effect, 1)
        control_discordance = _number(power.get("expectedControlDiscordance"), "development control discordance", 0, 1)
        design_effect = _number(power.get("designEffect"), "cluster design effect", 1, 100)
        z = NormalDist().inv_cdf(.975) + NormalDist().inv_cdf(target)
        required_broad = math.ceil(z * z * max(discordance - effect * effect, 0) / (effect * effect) * design_effect)
        margin = policy["noninferiorityMargin"]
        required_controls = math.ceil(z * z * control_discordance / (margin * margin) * design_effect) if margin else sys.maxsize
        artifacts.read(power.get("developmentEvidence"))
        comparisons["prospectivePower"] = dict(requiredBroadClusters=required_broad,
                                                requiredControlClusters=required_controls,
                                                method="approximate_paired_binary_normal_planning/1")
        finite = finite_ni_power(comparisons["exactDirected"]["clusters"], control_discordance, margin)
        comparisons["prospectivePower"]["finiteControlModel"] = finite
        if (primary["clusters"] < required_broad
                or comparisons["exactDirected"]["clusters"] < required_controls
                or finite["finiteBoundPassProbability"] < target):
            failures.append("registered prospective power requirement is not met")
        # A normal approximation does not prove power of our actual bootstrap
        # plus finite-risk acceptance rule. Require a frozen reviewed planning
        # artifact, including a separate broad precision distribution model.
        simulation_ref = power.get("acceptanceRuleSimulation")
        if not isinstance(simulation_ref, dict):
            failures.append("prospective simulation of actual acceptance rules is missing")
        else:
            simulation = artifacts.document(simulation_ref)
            if _timestamp(simulation.get("frozenAt")) >= started:
                raise ValueError("power acceptance-rule planning was frozen after collection")
            failures += _review_failures(simulation.get("independentReview"), "prospective acceptance-rule power")
            if simulation.get("schema") != "compass.broad-query-power-simulation/1":
                raise ValueError("unsupported prospective power simulation")
            for metric in ("broadSuccess", "exactDirected", "claimPrecisionEmptyZero"):
                design = _object(simulation.get("metrics", {}).get(metric), "prospective metric design")
                _text(design.get("assumptions"), "prospective distribution assumptions", 16384)
                _text(design.get("method"), "actual acceptance-rule simulation method", 4096)
                artifacts.read(design.get("code"))
                artifacts.read(design.get("results"))
                if (_number(design.get("powerLowerBound"), "planned power lower bound", 0, 1) < target
                        or _integer(design.get("independentClusters"), "planned independent clusters", 1, MAX_TASKS)
                        > comparisons[metric]["clusters"]):
                    failures.append(f"{metric}: prospectively reviewed acceptance-rule power is insufficient")
        if control_discordance == 0 and not isinstance(power.get("equalityAlternativeEvidence"), dict):
            failures.append("zero adverse probability requires pre-output equality-alternative evidence")
        elif control_discordance == 0:
            artifacts.read(power["equalityAlternativeEvidence"])
    # Reuse the existing graph-quality auditor, not a self-reported pass bit.
    graph_audits = _array(manifest.get("graphQualification", []), "graph qualification audits", 32)
    graph_claim_scope = set(registration.get("graphQualificationRepositories", repositories))
    qualified_graphs = set()
    if graph_audits:
        from benchmarks.performance.compass.audit import load_manifest, run_audit
        for entry in graph_audits:
            ref = entry.get("manifest")
            artifacts.read(ref)
            audit_manifest = load_manifest(artifacts.root / ref["path"])
            if "repository" in entry:
                # Compatibility convenience for a single-corpus production audit.
                repo = _choice(entry["repository"], repositories, "graph audit repository")
                graph_entry = next(g for g in graph_records if g["repository"] == repo and g["tool"] == "compass")
                graph_root = artifacts.root / graph_entry["artifact"]["path"]
                source_root = Path(manifest["sourceRoots"][repo]).resolve()
            else:
                # One aggregate production manifest preserves its existing
                # 2,000-total/400-per-corpus floors, rather than inventing a
                # requirement for 2,000 independent judgments per repository.
                graph_root = (artifacts.root / _relative(entry.get("graphRoot"))).resolve()
                if not graph_root.is_relative_to(artifacts.root):
                    raise ValueError("aggregate audit graph root escapes the evidence bundle")
                source_root = Path(_text(entry.get("sourceRoot"), "aggregate audit source root")).resolve()
            covered = set()
            for corpus in audit_manifest.corpora:
                repo = _choice(corpus.name, repositories, "production-audited repository")
                if repo not in graph_claim_scope:
                    raise ValueError("production audit falls outside the frozen graph quality claim scope")
                if repo in qualified_graphs or corpus.commit != repositories[repo]["commit"]:
                    raise ValueError("duplicate or differently pinned production graph audit")
                single = len(audit_manifest.corpora) == 1
                actual_source = source_root if single and corpus.path == "." else source_root / corpus.path
                actual_graph = graph_root if single and graph_root.is_file() else graph_root / corpus.graph
                graph_entry = next(g for g in graph_records if g["repository"] == repo and g["tool"] == "compass")
                if (actual_source.resolve() != Path(manifest["sourceRoots"][repo]).resolve()
                        or actual_graph.resolve() != (artifacts.root / graph_entry["artifact"]["path"]).resolve()
                        or corpus.graph_sha256 != graph_entry["artifact"]["sha256"]):
                    raise ValueError("production audit does not cover the exact captured source/graph")
                covered.add(repo)
            audited = run_audit(artifacts.root / ref["path"], graph_root, source_root)
            if audited.eligible_for_quality_claim:
                qualified_graphs.update(covered)
    if qualified_graphs != graph_claim_scope:
        failures.append("existing independent production graph-qualification gate is incomplete")
    # Repo/language strata expose regressions rather than hiding them in the aggregate.
    summaries = {}
    for tool in TOOLS:
        summaries[tool] = {}
        for arm in ARMS:
            selected = [r for (t, name, a), r in sorted(scored.items()) if name == tool and a == arm]
            summaries[tool][arm] = _summary(selected)
            summaries[tool][arm]["byRepository"] = {repo: _summary([r for r in selected if r["repository"] == repo]) for repo in sorted(repositories)}
            summaries[tool][arm]["byLanguage"] = {language: _summary([r for r in selected if repositories[r["repository"]]["language"] == language]) for language in sorted(languages)}
    for repo in sorted(repositories):
        a = summaries["compass"]["normalized"]["byRepository"][repo]["answerableSuccess"]
        b = summaries["graphify"]["normalized"]["byRepository"][repo]["answerableSuccess"]
        if a is not None and b is not None and a < b - .10:
            failures.append(f"{repo}: material subgroup regression requires independent investigation")
    _verify_sources(registration, manifest["sourceRoots"])
    artifacts.recheck()
    return dict(eligibleForSuperiorityClaim=not failures, failures=sorted(set(failures)),
                verificationBoundary="Programmatic integrity and scoring only; reviewer attestations are not authenticated and source semantics are not inferred by this tool. AI review is not human review.",
                reviewDisclosure=dict(sourceIndependentReviewerType=registration.get("independentReview", {}).get("reviewerType"),
                                      outputIndependentReviewerTypes=sorted({str(r.get("independentReview", {}).get("reviewerType")) for r in review_rows})),
                policy=policy, comparisons=comparisons, summaries=summaries,
                graphQualificationScope=sorted(graph_claim_scope), qualifiedGraphRepositories=sorted(qualified_graphs),
                queryOnlyControlRepositories=sorted(set(repositories) - graph_claim_scope),
                metricCohorts=cohorts, comparisonScope=dict(primaryArm="normalized", defaultArm="default",
                    compassView=arm_map["normalized"].get("compassView", "default"),
                    configuredComparison=arm_map["normalized"].get("configuredComparison", False)),
                rows=[dict(tool=tool, arm=arm, **row) for (_, tool, arm), row in sorted(scored.items())],
                costs=sorted(costs, key=lambda c: (c["task"], c["tool"], c["arm"])),
                costSummaries={tool: {arm: _cost_summary([c for c in costs if c["tool"] == tool and c["arm"] == arm])
                                      for arm in ARMS} for tool in TOOLS},
                artifacts={p: digest for p, digest in sorted(artifacts.seen.items())})


def recompute_report(report: dict) -> dict:
    """Verify a saved report through the same path as the public --check command."""
    if report.get("schema") != REPORT_SCHEMA:
        raise ValueError("unsupported broad-query review report schema")
    if report.get("assessorSha256") != sha(read_bounded(Path(__file__), MAX_JSON_BYTES)):
        raise ValueError("assessor changed since report generation; regenerate the report")
    input_path = Path(_text(report.get("inputPath"), "input manifest path", 4096))
    data = read_bounded(input_path, MAX_JSON_BYTES)
    if sha(data) != report.get("inputSha256"):
        raise ValueError("input manifest changed since scoring")
    assessment = assess(_json(data), input_path.parent)
    if assessment != report.get("assessment"):
        raise ValueError("stored metrics or claim eligibility were fabricated or are stale")
    return assessment


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    actions = parser.add_mutually_exclusive_group(required=True)
    actions.add_argument("--self-test", action="store_true")
    actions.add_argument("--validate-registration", type=Path)
    actions.add_argument("--score", type=Path, help="immutable input manifest; emit a diagnostic report")
    actions.add_argument("--check", type=Path, help="recompute a report and require every qualification gate")
    parser.add_argument("--output", type=Path, help="new report path, required with --score")
    args = parser.parse_args(argv)
    try:
        if args.self_test:
            import unittest
            suite = unittest.defaultTestLoader.loadTestsFromName("benchmarks.agent_query.tests.test_broad_query_review")
            return 0 if unittest.TextTestRunner(verbosity=1).run(suite).wasSuccessful() else 1
        if args.validate_registration:
            registration = validate_registration(_json(read_bounded(args.validate_registration, MAX_JSON_BYTES)))
            print(json.dumps(dict(valid=True, tasks=len(registration["tasks"]),
                                  qualificationEstablished=False, note="Shape validation is not independent source review.")))
            return 0
        if args.score:
            if args.output is None:
                parser.error("--score requires --output")
            manifest_bytes = read_bounded(args.score, MAX_JSON_BYTES)
            assessment = assess(_json(manifest_bytes), args.score.parent)
            # Keep all references relative to the input root. The report can live
            # elsewhere, but --check never resolves references relative to itself.
            report = dict(schema=REPORT_SCHEMA, inputPath=str(args.score.resolve()),
                          inputSha256=sha(manifest_bytes), assessorSha256=sha(read_bounded(Path(__file__), MAX_JSON_BYTES)),
                          assessment=assessment)
            args.output.parent.mkdir(parents=True, exist_ok=True)
            encoded = (json.dumps(report, indent=2, sort_keys=True, allow_nan=False) + "\n").encode()
            if len(encoded) > MAX_JSON_BYTES:
                raise ValueError("report exceeds the bounded JSON artifact limit")
            from benchmarks.agent_query.broad_query_oracle import publish_exclusive
            publish_exclusive(args.output, encoded)
            print(json.dumps(dict(report=str(args.output), eligibleForSuperiorityClaim=assessment["eligibleForSuperiorityClaim"],
                                  failures=assessment["failures"])))
            return 0  # Scoring diagnostics is useful even when qualification fails.
        report = _json(read_bounded(args.check, MAX_JSON_BYTES))
        assessment = recompute_report(report)
        print(json.dumps(dict(eligibleForSuperiorityClaim=assessment["eligibleForSuperiorityClaim"], failures=assessment["failures"])))
        return 0 if assessment["eligibleForSuperiorityClaim"] else 1
    except (OSError, ValueError, TypeError, KeyError, IndexError, RecursionError) as error:
        print(f"broad-query review rejected: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
