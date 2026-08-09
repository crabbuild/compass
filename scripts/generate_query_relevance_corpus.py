#!/usr/bin/env python3
"""Generate the reviewed executable Compass query-relevance corpus.

The checked-in artifact is generated from the equivalence classes below so
reviewers can audit the semantic judgment once per class and still execute a
large paraphrase matrix. This script never derives an expected answer from
Compass output.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / "crates/compass-query/tests/fixtures/relevance/executable-reviewed-v2.json"
SCHEMA = "compass.query-judgments/1"
REVIEW = (
    "AI-reviewed synthetic equivalence case; approved by Codex judgment on "
    "2026-08-08; not production telemetry"
)


def edge(source: str, target: str, kind: str) -> dict[str, object]:
    return {
        "edge": {
            "source": source,
            "target": target,
            "kind": kind,
            "direction": "source_to_target",
        },
        "grade": 3,
    }


def record(
    identifier: str,
    text: str,
    query_class: str,
    intent: str,
    *,
    nodes: tuple[tuple[str, int], ...] = (),
    edges: tuple[dict[str, object], ...] = (),
    paths: tuple[dict[str, object], ...] = (),
    ambiguity: tuple[str, ...] = (),
    must_not_return: tuple[str, ...] = (),
    family: str,
) -> dict[str, object]:
    return {
        "id": identifier,
        "text": text,
        "class": query_class,
        "locale": "en-US",
        "expectedIntent": intent,
        "expectedSlots": {"operation": intent, "truncated": "false"},
        "nodeJudgments": [{"id": node, "grade": grade} for node, grade in nodes],
        "edgeJudgments": list(edges),
        "pathJudgments": list(paths),
        "acceptableAmbiguity": list(ambiguity),
        "mustNotReturn": list(must_not_return),
        "notes": f"{REVIEW}; family={family}",
    }


def search_records() -> list[dict[str, object]]:
    templates = (
        "find definition of {symbol}",
        "definition of {symbol}",
        "search for {symbol}",
        "show me {symbol}",
        "where is {symbol} defined",
        "find {symbol}",
        "search {symbol}",
        "show {symbol}",
    )
    targets = (
        ("UserService.list", "n:list", "exact"),
        ("Menu.café", "n:unicode", "exact"),
        ("café", "n:unicode", "lexical"),
        ("cafe", "n:unicode", "lexical"),
        ("Profile.résumé", "n:resume", "exact"),
        ("résumé", "n:resume", "lexical"),
        ("resume", "n:resume", "lexical"),
        ("Units.Ångström", "n:unicode-case", "exact"),
        ("Ångström", "n:unicode-case", "lexical"),
        ("ångström", "n:unicode-case", "lexical"),
        ("Cache.cache_key", "n:snake", "exact"),
        ("cache_key", "n:snake", "lexical"),
        ("Api.fetchUserRecord", "n:camel", "exact"),
        ("fetchUserRecord", "n:camel", "lexical"),
        ("UserService.listing", "n:listing", "exact"),
        ("listing", "n:listing", "lexical"),
        ("PaymentGateway.charge", "n:z-payment-charge", "exact"),
        ("paymentgateway.charge", "n:z-payment-charge", "exact"),
        ("charge", "n:z-payment-charge", "intent"),
        ("GeneratedPaymentGateway.charge", "n:a-generated-charge", "exact"),
    )
    result: list[dict[str, object]] = []
    ordinal = 1
    for symbol, expected, query_class in targets:
        for template in templates:
            nodes = [(expected, 3)]
            if symbol == "charge":
                nodes.append(("n:a-generated-charge", 1))
            result.append(
                record(
                    f"search-{ordinal:03d}",
                    template.format(symbol=symbol),
                    query_class,
                    "search",
                    nodes=tuple(nodes),
                    family="search-normalization",
                )
            )
            ordinal += 1

    for typo, expected in (
        ("litsing", "n:listing"),
        ("cace_key", "n:snake"),
        ("fetchUserRecrod", "n:camel"),
        ("PaymntGateway.charge", "n:z-payment-charge"),
    ):
        for template in templates:
            result.append(
                record(
                    f"fuzzy-{ordinal:03d}",
                    template.format(symbol=typo),
                    "fuzzy",
                    "search",
                    nodes=((expected, 3),),
                    family="single-edit-fuzzy-recovery",
                )
            )
            ordinal += 1
    return result


def callers_records() -> list[dict[str, object]]:
    templates = (
        "find callers of {symbol}",
        "show callers of {symbol}",
        "callers of {symbol}",
        "who calls {symbol}",
        "what calls {symbol}",
        "what functions call {symbol}",
        "what methods call {symbol}",
        "which functions call {symbol}",
        "which methods call {symbol}",
        "where is {symbol} called",
    )
    nodes = (("n:caller", 3), ("n:list", 3), ("n:route", 3))
    edges = (
        edge("n:caller", "n:list", "calls"),
        edge("n:route", "n:list", "routes_to"),
    )
    return [
        record(
            f"callers-{index:03d}",
            template.format(symbol=symbol) + punctuation,
            "edge",
            "callers",
            nodes=nodes,
            edges=edges,
            family="incoming-relation-recall",
        )
        for index, (template, symbol, punctuation) in enumerate(
            (
                (template, symbol, punctuation)
                for template in templates
                for symbol in ("UserService.list", "userservice.list", "UserService.lsit")
                for punctuation in ("", "?")
            ),
            start=1,
        )
    ]


def callees_records() -> list[dict[str, object]]:
    templates = (
        "find callees of {symbol}",
        "show callees of {symbol}",
        "callees of {symbol}",
        "calls made by {symbol}",
        "what does {symbol} call",
        "what does {symbol} invoke",
        "what functions does {symbol} call",
        "what functions does {symbol} invoke",
        "what methods does {symbol} call",
        "what methods does {symbol} invoke",
    )
    nodes = (("n:caller", 3), ("n:list", 3))
    edges = (edge("n:caller", "n:list", "calls"),)
    return [
        record(
            f"callees-{index:03d}",
            template.format(symbol=symbol) + punctuation,
            "edge",
            "callees",
            nodes=nodes,
            edges=edges,
            family="outgoing-call-recall",
        )
        for index, (template, symbol, punctuation) in enumerate(
            (
                (template, symbol, punctuation)
                for template in templates
                for symbol in ("Api.caller", "api.caller", "Api.callar")
                for punctuation in ("", "?")
            ),
            start=1,
        )
    ]


def impact_records() -> list[dict[str, object]]:
    templates = (
        "what is impacted by {symbol}",
        "what depends on {symbol}",
        "who depends on {symbol}",
        "what breaks if {symbol}",
        "what changes if {symbol}",
        "dependents of {symbol}",
        "impact of {symbol}",
        "what is the impact of {symbol}",
        "what would break if {symbol}",
        "what breaks if {symbol} changes",
    )
    nodes = (("n:caller", 3), ("n:dependent", 3))
    edges = (edge("n:dependent", "n:caller", "imports"),)
    return [
        record(
            f"impact-{index:03d}",
            template.format(symbol=symbol) + punctuation,
            "intent",
            "impact",
            nodes=nodes,
            edges=edges,
            family="downstream-impact-recall",
        )
        for index, (template, symbol, punctuation) in enumerate(
            (
                (template, symbol, punctuation)
                for template in templates
                for symbol in ("Api.caller", "api.caller", "Api.callar")
                for punctuation in ("", "?")
            ),
            start=1,
        )
    ]


def path_records() -> list[dict[str, object]]:
    templates = (
        "shortest path from {source} to {target}",
        "path from {source} to {target}",
        "route from {source} to {target}",
        "connection from {source} to {target}",
        "how does {source} reach {target}",
        "how can {source} reach {target}",
        "how is {source} connected to {target}",
    )
    pairs = (
        ("Api.caller", "Store.callee"),
        ("api.caller", "store.callee"),
        ("Api.callar", "Store.callee"),
        ("Api.caller", "Store.calee"),
        ("Api.callar", "Store.calee"),
    )
    variants = [
        (template, source, target, punctuation)
        for template in templates
        for source, target in pairs
        for punctuation in ("", "?")
    ][:50]
    return [
        record(
            f"path-{index:03d}",
            template.format(source=source, target=target) + punctuation,
            "path",
            "node_trail",
            nodes=(("n:caller", 3), ("n:list", 3), ("n:callee", 3)),
            edges=(
                edge("n:caller", "n:list", "calls"),
                edge("n:list", "n:callee", "calls"),
            ),
            paths=(
                {
                    "pattern": {
                        "edgeKinds": ["calls", "calls"],
                        "endpointIds": ["n:caller", "n:callee"],
                    },
                    "grade": 3,
                },
            ),
            family="directed-path-recall",
        )
        for index, (template, source, target, punctuation) in enumerate(variants, start=1)
    ]


def architecture_records() -> list[dict[str, object]]:
    templates = (
        "find definition of {symbol}",
        "definition of {symbol}",
        "search for {symbol}",
        "show me {symbol}",
        "find {symbol}",
        "show {symbol}",
    )
    punctuation = ("", "?", "!", ".", "   ?")
    return [
        record(
            f"architecture-{index:03d}",
            template.format(symbol="express::GET::/users") + suffix,
            "architecture",
            "search",
            nodes=(("n:route", 3),),
            family="framework-route-discovery",
        )
        for index, (template, suffix) in enumerate(
            ((template, suffix) for template in templates for suffix in punctuation), start=1
        )
    ]


def negative_records() -> list[dict[str, object]]:
    templates = (
        "find definition of {symbol}",
        "definition of {symbol}",
        "search for {symbol}",
        "show me {symbol}",
        "where is {symbol} defined",
        "find {symbol}",
        "search {symbol}",
        "show {symbol}",
    )
    symbols = (
        "definitely_missing",
        "KafkaConsumerLagMonitor",
        "BillingReconcilerV9",
        "Nonexistent::CircuitBreaker",
        "missingFeatureFlagResolver",
    )
    forbidden = (
        "n:list",
        "n:listing",
        "n:caller",
        "n:callee",
        "n:z-payment-charge",
    )
    return [
        record(
            f"negative-{index:03d}",
            template.format(symbol=symbol),
            "negative",
            "search",
            must_not_return=forbidden,
            family="reviewed-no-answer",
        )
        for index, (symbol, template) in enumerate(
            ((symbol, template) for symbol in symbols for template in templates), start=1
        )
    ]


def ambiguity_records() -> list[dict[str, object]]:
    templates = (
        "find definition of charge?",
        "definition of charge?",
        "search for charge?",
        "show me charge?",
        "where is charge defined?",
        "find charge?",
        "search charge?",
        "show charge?",
    )
    return [
        record(
            f"ambiguity-{index:03d}",
            text,
            "intent",
            "search",
            nodes=(("n:z-payment-charge", 3), ("n:a-generated-charge", 1)),
            ambiguity=("n:a-generated-charge",),
            family="source-over-generated-ambiguity",
        )
        for index, text in enumerate(templates, start=1)
    ]


def build() -> dict[str, object]:
    queries = (
        search_records()
        + callers_records()
        + callees_records()
        + impact_records()
        + path_records()
        + architecture_records()
        + negative_records()
        + ambiguity_records()
    )
    if len(queries) != 500:
        raise RuntimeError(f"expected 500 generated judgments, got {len(queries)}")
    ids = [query["id"] for query in queries]
    texts = [query["text"] for query in queries]
    if len(ids) != len(set(ids)):
        raise RuntimeError("generated query ids are not unique")
    if len(texts) != len(set(texts)):
        raise RuntimeError("generated query texts are not unique")
    return {
        "schema": SCHEMA,
        "corpusId": "compass-query-executable-ai-reviewed-v2",
        "graphSchema": "compass.graph/1",
        "graphDigest": "sha256:ac93d0a2a2d25d3d089e1f6eccab2e90246a045bac23c04fd0cfcc3d4125cf2b",
        "repositoryRevision": "crates/compass-query/tests/support@v2",
        "analyzerVersion": "compass.search-term/1",
        "queries": queries,
    }


def encoded() -> str:
    return json.dumps(build(), ensure_ascii=False, indent=2, sort_keys=False) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    expected = encoded()
    if args.check:
        if not OUTPUT.exists() or OUTPUT.read_text(encoding="utf-8") != expected:
            print(f"{OUTPUT.relative_to(ROOT)} is stale; regenerate it with {Path(__file__).name}")
            return 1
        return 0
    OUTPUT.write_text(expected, encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
