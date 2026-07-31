"""Deterministic, source-grounded code-graph quality auditing."""

from __future__ import annotations

from collections import Counter, defaultdict
from dataclasses import dataclass
import hashlib
import json
import math
from pathlib import Path, PurePosixPath
import re
import sqlite3
import subprocess
from typing import Any, Iterable

from .model import (
    AuditCapabilityIdentity,
    AuditCorpus,
    AuditEndpoint,
    AuditGraphFact,
    AuditManifest,
    AuditMetric,
    AuditOccurrence,
    AuditRecord,
    AuditResult,
    WilsonInterval,
    to_json_value,
)
from .correctness import (
    _classify_edges,
    _classify_nodes,
    _edge_facts,
    _node_facts,
)
from .occurrences import SourceOccurrenceOracle


AUDIT_SCHEMA = "compass.quality-audit"
AUDIT_RESULT_SCHEMA = "compass.quality-audit-result"
QUALIFICATION_MINIMUM = 2_000
CORPUS_MINIMUM = 400
RELATION_MINIMUM = 100
CAPABILITY_MINIMUM = 100
TARGET_CLUSTER_MAXIMUM_FRACTION = 0.10
PRECISION_GATE = 0.995
PRECISION_WILSON_LOWER_GATE = 0.99
CAPABILITY_PRECISION_GATE = 0.99
CAPABILITY_RECALL_GATE = 0.95
WILSON_95_Z = 1.959963984540054

POOLS = frozenset(("accepted", "source_oracle", "graphify_hypothesis"))
JUDGMENTS = frozenset(
    (
        "correct",
        "invalid",
        "ambiguous",
        "external",
        "represented_elsewhere",
        "missing",
        "fabricated_occurrence",
        "cross_language_match",
        "unsafe_local_substitution",
    )
)
CRITICAL_JUDGMENTS = (
    "fabricated_occurrence",
    "cross_language_match",
    "unsafe_local_substitution",
)
CORRECT_ACCEPTED_JUDGMENTS = frozenset(
    ("correct", "external", "represented_elsewhere")
)
RECALL_RECOVERED_JUDGMENTS = CORRECT_ACCEPTED_JUDGMENTS
RECALL_TRUTH_JUDGMENTS = frozenset(
    ("correct", "external", "represented_elsewhere", "missing")
)
HEX_40 = re.compile(r"^[0-9a-f]{40}$")
HEX_64 = re.compile(r"^[0-9a-f]{64}$")
IDENTITY = re.compile(r"^[a-z0-9][a-z0-9_.:/-]*$")


class AuditError(ValueError):
    """An audit input is stale, unsafe, or structurally invalid."""


@dataclass(frozen=True)
class _GraphIndex:
    nodes: dict[str, dict[str, Any]]
    facts: dict[tuple[str, str, str], tuple[dict[str, Any], ...]]


def _source_line_range(root: Path, source_file: str, location: str) -> tuple[int, int, str] | None:
    match = re.fullmatch(r"L([1-9][0-9]*)", location)
    if match is None:
        return None
    relative = Path(source_file.replace("\\", "/"))
    if relative.is_absolute():
        return None
    root = root.resolve()
    path = (root / relative).resolve()
    try:
        path.relative_to(root)
    except ValueError:
        return None
    try:
        contents = path.read_bytes()
    except OSError:
        return None
    lines = contents.splitlines(keepends=True)
    line = int(match.group(1))
    if line > len(lines):
        return None
    start = sum(map(len, lines[: line - 1]))
    snippet = lines[line - 1].rstrip(b"\r\n")
    if not snippet:
        return None
    end = start + len(snippet)
    normalized = snippet.replace(b"\r\n", b"\n")
    return start, end, hashlib.sha256(normalized).hexdigest()


def _capability_for_relation(relation: str) -> str:
    return {
        "calls": "calls",
        "contains": "ownership",
        "exports": "reexports",
        "extends": "base_types",
        "imports": "imports",
        "rationale_for": "rationale",
        "references": "type_references",
        "routes_to": "routes",
    }.get(relation, relation)


def _target_cluster(label: str, identifier: str) -> str:
    normalized = re.sub(r"[^a-z0-9_.:/-]+", "-", label.casefold()).strip("-")
    if normalized and IDENTITY.fullmatch(normalized) is not None:
        return normalized[:120]
    return f"target-{hashlib.sha256(identifier.encode()).hexdigest()[:16]}"


def export_comparison_candidates(
    database_path: Path,
    graph_path: Path,
    corpus_root: Path,
    corpus: str,
    adapter: str,
    destination: Path,
) -> Path:
    """Export deterministic, unjudged source-bounded comparison hypotheses."""

    corpus = _text(corpus, "corpus", identity=True)
    adapter = _text(adapter, "adapter", identity=True)
    if not database_path.is_file():
        raise AuditError(f"comparison database does not exist: {database_path}")
    if not graph_path.is_file():
        raise AuditError(f"graph does not exist: {graph_path}")
    corpus_root = corpus_root.resolve()
    if not corpus_root.is_dir():
        raise AuditError(f"corpus root does not exist: {corpus_root}")

    with sqlite3.connect(database_path) as database:
        compass_nodes = _node_facts(database, "compass")
        graphify_nodes = _node_facts(database, "graphify")
        node_coverage, node_mapping = _classify_nodes(graphify_nodes, compass_nodes)
        compass_edges = _edge_facts(database, "compass")
        graphify_edges = _edge_facts(database, "graphify")
        coverage = _classify_edges(
            graphify_edges,
            compass_edges,
            graphify_nodes,
            compass_nodes,
            node_coverage,
            node_mapping,
            SourceOccurrenceOracle(corpus_root),
        )

    compass_by_payload = {edge.payload_sha256: edge for edge in compass_edges}
    candidates: list[dict[str, Any]] = []
    for graphify, classification in zip(graphify_edges, coverage, strict=True):
        bounded = _source_line_range(
            corpus_root,
            graphify.occurrence_file,
            graphify.occurrence_location,
        )
        if bounded is None:
            continue
        start, end, snippet_sha256 = bounded
        compass_fact = (
            compass_by_payload.get(classification.compass_fact)
            if classification.compass_fact is not None
            else None
        )
        source_id = (
            compass_fact.source
            if compass_fact is not None
            else node_mapping.get(graphify.source)
        )
        if source_id is None:
            continue
        target_id = (
            compass_fact.target
            if compass_fact is not None
            else node_mapping.get(graphify.target, graphify.target)
        )
        source_node = compass_nodes.get(source_id) or graphify_nodes.get(graphify.source)
        target_node = compass_nodes.get(target_id) or graphify_nodes.get(graphify.target)
        if source_node is None or target_node is None:
            continue
        language = source_node.language
        if not language:
            continue
        identity = (
            corpus,
            graphify.relation,
            source_id,
            target_id,
            graphify.occurrence_file,
            graphify.occurrence_location,
            classification.status,
            classification.reason,
        )
        candidate_id = "candidate-" + hashlib.sha256(
            json.dumps(identity, separators=(",", ":")).encode()
        ).hexdigest()[:24]
        candidates.append(
            {
                "id": candidate_id,
                "suggestedPool": (
                    "accepted"
                    if classification.status in {"exact", "dominated"}
                    and compass_fact is not None
                    else "graphify_hypothesis"
                ),
                "adapter": adapter,
                "capability": _capability_for_relation(graphify.relation),
                "language": language,
                "relation": graphify.relation,
                "confidence": classification.status,
                "targetCluster": _target_cluster(
                    target_node.qualified_name or target_node.normalized_label,
                    target_id,
                ),
                "source": {"nodeId": source_id, "language": language},
                "target": {
                    "nodeId": target_id,
                    "language": target_node.language or language,
                },
                "occurrence": {
                    "file": graphify.occurrence_file,
                    "line": int(graphify.occurrence_location[1:]),
                    "startByte": start,
                    "endByte": end,
                    "snippetSha256": snippet_sha256,
                    "requiresExactGraphRange": True,
                },
                "comparison": {
                    "status": classification.status,
                    "reason": classification.reason,
                    "compassFact": classification.compass_fact,
                },
                "judgment": None,
                "reason": None,
            }
        )
    candidates.sort(key=lambda candidate: candidate["id"])
    payload = {
        "schema": "compass.quality-audit-candidates",
        "corpus": {
            "name": corpus,
            "commit": _corpus_commit(corpus_root),
            "path": str(corpus_root),
            "graph": str(graph_path.resolve()),
            "graphSha256": _file_sha256(graph_path),
        },
        "adapter": adapter,
        "recordsAreUnjudged": True,
        "candidates": candidates,
    }
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text(
        json.dumps(payload, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
        + "\n",
        encoding="utf-8",
    )
    return destination


def _expect_object(value: object, context: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise AuditError(f"{context} must be an object")
    return value


def _expect_array(value: object, context: str) -> list[Any]:
    if not isinstance(value, list):
        raise AuditError(f"{context} must be an array")
    return value


def _keys(
    value: dict[str, Any],
    *,
    required: Iterable[str],
    optional: Iterable[str] = (),
    context: str,
) -> None:
    required_set = set(required)
    allowed = required_set | set(optional)
    missing = sorted(required_set - value.keys())
    unknown = sorted(value.keys() - allowed)
    if missing:
        raise AuditError(f"{context} missing fields: {', '.join(missing)}")
    if unknown:
        raise AuditError(f"{context} has unknown fields: {', '.join(unknown)}")


def _text(value: object, context: str, *, identity: bool = False) -> str:
    if not isinstance(value, str) or not value.strip():
        raise AuditError(f"{context} must be a non-empty string")
    if identity and IDENTITY.fullmatch(value) is None:
        raise AuditError(f"{context} is not a stable lowercase identity")
    return value


def _safe_path(value: object, context: str, *, allow_dot: bool = False) -> str:
    text = _text(value, context)
    if "\\" in text:
        raise AuditError(f"{context} must use forward slashes")
    path = PurePosixPath(text)
    if path.is_absolute() or ".." in path.parts:
        raise AuditError(f"{context} must be a safe relative path")
    if text == ".":
        if allow_dot:
            return text
        raise AuditError(f"{context} must identify a file")
    if any(part in {"", "."} for part in path.parts):
        raise AuditError(f"{context} must be normalized")
    return text


def _sha256(value: object, context: str) -> str:
    text = _text(value, context)
    if HEX_64.fullmatch(text) is None:
        raise AuditError(f"{context} must be a 64-character lowercase SHA-256")
    return text


def _commit(value: object, context: str) -> str:
    text = _text(value, context)
    if HEX_40.fullmatch(text) is None:
        raise AuditError(f"{context} must be a 40-character lowercase commit")
    return text


def _endpoint(value: object, context: str) -> AuditEndpoint:
    item = _expect_object(value, context)
    _keys(item, required=("nodeId", "language"), context=context)
    return AuditEndpoint(
        node_id=_text(item["nodeId"], f"{context}.nodeId"),
        language=_text(item["language"], f"{context}.language", identity=True),
    )


def _occurrence(value: object, context: str) -> AuditOccurrence:
    item = _expect_object(value, context)
    _keys(
        item,
        required=("file", "startByte", "endByte", "snippetSha256"),
        context=context,
    )
    start = item["startByte"]
    end = item["endByte"]
    if (
        isinstance(start, bool)
        or not isinstance(start, int)
        or isinstance(end, bool)
        or not isinstance(end, int)
        or start < 0
        or end <= start
    ):
        raise AuditError(f"{context} must contain a positive non-empty byte range")
    return AuditOccurrence(
        file=_safe_path(item["file"], f"{context}.file"),
        start_byte=start,
        end_byte=end,
        snippet_sha256=_sha256(item["snippetSha256"], f"{context}.snippetSha256"),
    )


def _graph_fact(value: object, context: str) -> AuditGraphFact:
    item = _expect_object(value, context)
    _keys(item, required=("source", "target", "relation"), context=context)
    return AuditGraphFact(
        source=_text(item["source"], f"{context}.source"),
        target=_text(item["target"], f"{context}.target"),
        relation=_text(item["relation"], f"{context}.relation", identity=True),
    )


def _corpus(value: object, index: int) -> AuditCorpus:
    context = f"corpora[{index}]"
    item = _expect_object(value, context)
    _keys(
        item,
        required=("name", "commit", "path", "graph", "graphSha256"),
        context=context,
    )
    return AuditCorpus(
        name=_text(item["name"], f"{context}.name", identity=True),
        commit=_commit(item["commit"], f"{context}.commit"),
        path=_safe_path(item["path"], f"{context}.path", allow_dot=True),
        graph=_safe_path(item["graph"], f"{context}.graph"),
        graph_sha256=_sha256(item["graphSha256"], f"{context}.graphSha256"),
    )


def _capability(value: object, index: int) -> AuditCapabilityIdentity:
    context = f"advertisedCapabilities[{index}]"
    item = _expect_object(value, context)
    _keys(
        item,
        required=("adapter", "capability"),
        optional=("frameworkPack",),
        context=context,
    )
    framework_pack = item.get("frameworkPack")
    return AuditCapabilityIdentity(
        adapter=_text(item["adapter"], f"{context}.adapter", identity=True),
        capability=_text(item["capability"], f"{context}.capability", identity=True),
        framework_pack=(
            _text(framework_pack, f"{context}.frameworkPack", identity=True)
            if framework_pack is not None
            else None
        ),
    )


def _record(value: object, index: int) -> AuditRecord:
    context = f"records[{index}]"
    item = _expect_object(value, context)
    _keys(
        item,
        required=(
            "id",
            "corpus",
            "pool",
            "adapter",
            "capability",
            "language",
            "relation",
            "confidence",
            "targetCluster",
            "source",
            "target",
            "occurrence",
            "judgment",
            "reason",
        ),
        optional=("frameworkPack", "representation"),
        context=context,
    )
    pool = _text(item["pool"], f"{context}.pool", identity=True)
    if pool not in POOLS:
        raise AuditError(f"{context}.pool is unknown: {pool}")
    judgment = _text(item["judgment"], f"{context}.judgment", identity=True)
    if judgment not in JUDGMENTS:
        raise AuditError(f"{context}.judgment is unknown: {judgment}")
    allowed = {
        "accepted": CORRECT_ACCEPTED_JUDGMENTS
        | frozenset(("invalid",))
        | frozenset(CRITICAL_JUDGMENTS),
        "source_oracle": RECALL_TRUTH_JUDGMENTS | frozenset(("ambiguous",)),
        "graphify_hypothesis": JUDGMENTS,
    }[pool]
    if judgment not in allowed:
        raise AuditError(f"{context} judgment {judgment!r} is invalid for pool {pool!r}")
    representation = item.get("representation")
    if judgment == "represented_elsewhere" and representation is None:
        raise AuditError(f"{context}.representation is required for represented_elsewhere")
    if judgment != "represented_elsewhere" and representation is not None:
        raise AuditError(f"{context}.representation is only valid for represented_elsewhere")
    source = _endpoint(item["source"], f"{context}.source")
    language = _text(item["language"], f"{context}.language", identity=True)
    if source.language != language:
        raise AuditError(f"{context}.source.language must equal record language")
    framework_pack = item.get("frameworkPack")
    return AuditRecord(
        record_id=_text(item["id"], f"{context}.id", identity=True),
        corpus=_text(item["corpus"], f"{context}.corpus", identity=True),
        pool=pool,
        adapter=_text(item["adapter"], f"{context}.adapter", identity=True),
        framework_pack=(
            _text(framework_pack, f"{context}.frameworkPack", identity=True)
            if framework_pack is not None
            else None
        ),
        capability=_text(item["capability"], f"{context}.capability", identity=True),
        language=language,
        relation=_text(item["relation"], f"{context}.relation", identity=True),
        confidence=_text(item["confidence"], f"{context}.confidence", identity=True),
        target_cluster=_text(
            item["targetCluster"], f"{context}.targetCluster", identity=True
        ),
        source=source,
        target=_endpoint(item["target"], f"{context}.target"),
        occurrence=_occurrence(item["occurrence"], f"{context}.occurrence"),
        judgment=judgment,
        reason=_text(item["reason"], f"{context}.reason"),
        representation=(
            _graph_fact(representation, f"{context}.representation")
            if representation is not None
            else None
        ),
    )


def load_manifest(path: Path) -> AuditManifest:
    try:
        raw = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise AuditError(f"could not load audit manifest {path}: {error}") from error
    item = _expect_object(raw, "manifest")
    _keys(
        item,
        required=(
            "schema",
            "mode",
            "corpora",
            "advertisedCapabilities",
            "requiredRelations",
            "records",
        ),
        context="manifest",
    )
    if item["schema"] != AUDIT_SCHEMA:
        raise AuditError(f"manifest schema must be {AUDIT_SCHEMA!r}")
    mode = _text(item["mode"], "manifest.mode", identity=True)
    if mode not in {"conformance", "qualification"}:
        raise AuditError("manifest.mode must be conformance or qualification")
    corpora = tuple(
        _corpus(value, index)
        for index, value in enumerate(_expect_array(item["corpora"], "manifest.corpora"))
    )
    if not corpora:
        raise AuditError("manifest.corpora must not be empty")
    if len({corpus.name for corpus in corpora}) != len(corpora):
        raise AuditError("manifest contains duplicate corpus names")
    capabilities = tuple(
        _capability(value, index)
        for index, value in enumerate(
            _expect_array(
                item["advertisedCapabilities"], "manifest.advertisedCapabilities"
            )
        )
    )
    if not capabilities:
        raise AuditError("manifest.advertisedCapabilities must not be empty")
    capability_keys = [
        (entry.adapter, entry.framework_pack, entry.capability) for entry in capabilities
    ]
    if capability_keys != sorted(set(capability_keys)):
        raise AuditError(
            "manifest.advertisedCapabilities must be sorted and contain no duplicates"
        )
    relations = tuple(
        _text(value, f"requiredRelations[{index}]", identity=True)
        for index, value in enumerate(
            _expect_array(item["requiredRelations"], "manifest.requiredRelations")
        )
    )
    if not relations or list(relations) != sorted(set(relations)):
        raise AuditError("manifest.requiredRelations must be sorted and contain no duplicates")
    records = tuple(
        _record(value, index)
        for index, value in enumerate(_expect_array(item["records"], "manifest.records"))
    )
    record_ids = [record.record_id for record in records]
    if len(record_ids) != len(set(record_ids)):
        raise AuditError("manifest contains duplicate record IDs")
    corpus_names = {corpus.name for corpus in corpora}
    advertised = {
        (entry.adapter, entry.framework_pack, entry.capability) for entry in capabilities
    }
    for record in records:
        if record.corpus not in corpus_names:
            raise AuditError(
                f"record {record.record_id!r} references unknown corpus {record.corpus!r}"
            )
        capability_key = (
            record.adapter,
            record.framework_pack,
            record.capability,
        )
        if capability_key not in advertised:
            raise AuditError(
                f"record {record.record_id!r} references unadvertised capability "
                f"{capability_key!r}"
            )
        if record.relation not in relations:
            raise AuditError(
                f"record {record.record_id!r} references undeclared relation "
                f"{record.relation!r}"
            )
    return AuditManifest(
        schema=AUDIT_SCHEMA,
        mode=mode,
        corpora=corpora,
        advertised_capabilities=capabilities,
        required_relations=relations,
        records=records,
    )


def wilson_interval(successes: int, total: int) -> WilsonInterval | None:
    if successes < 0 or total < 0 or successes > total:
        raise AuditError("Wilson counts must satisfy 0 <= successes <= total")
    if total == 0:
        return None
    observed = successes / total
    z_squared = WILSON_95_Z * WILSON_95_Z
    denominator = 1.0 + z_squared / total
    center = (observed + z_squared / (2.0 * total)) / denominator
    margin = (
        WILSON_95_Z
        * math.sqrt(
            observed * (1.0 - observed) / total
            + z_squared / (4.0 * total * total)
        )
        / denominator
    )
    return WilsonInterval(max(0.0, center - margin), min(1.0, center + margin))


def _metric(numerator: int, denominator: int, *, interval: bool = False) -> AuditMetric:
    return AuditMetric(
        numerator=numerator,
        denominator=denominator,
        observed=(numerator / denominator if denominator else None),
        wilson_95=wilson_interval(numerator, denominator) if interval else None,
    )


def _file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def _corpus_commit(root: Path) -> str:
    marker = root / ".compass-audit-commit"
    if marker.is_file():
        return _commit(marker.read_text(encoding="utf-8").strip(), str(marker))
    completed = subprocess.run(
        ("git", "-C", str(root), "rev-parse", "HEAD"),
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8",
    )
    if completed.returncode != 0:
        detail = completed.stderr.strip() or "not a Git checkout"
        raise AuditError(f"could not identify corpus revision at {root}: {detail}")
    return _commit(completed.stdout.strip(), f"corpus revision at {root}")


def _index_graph(path: Path) -> _GraphIndex:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise AuditError(f"could not load graph {path}: {error}") from error
    root = _expect_object(value, f"graph {path}")
    nodes: dict[str, dict[str, Any]] = {}
    for index, raw in enumerate(_expect_array(root.get("nodes"), f"graph {path}.nodes")):
        node = _expect_object(raw, f"graph {path}.nodes[{index}]")
        node_id = _text(node.get("id"), f"graph {path}.nodes[{index}].id")
        if node_id in nodes:
            raise AuditError(f"graph {path} has duplicate node ID {node_id!r}")
        nodes[node_id] = node
    facts: dict[tuple[str, str, str], list[dict[str, Any]]] = defaultdict(list)
    links = root.get("links", root.get("edges"))
    for index, raw in enumerate(_expect_array(links, f"graph {path}.links")):
        edge = _expect_object(raw, f"graph {path}.links[{index}]")
        source = _text(edge.get("source"), f"graph {path}.links[{index}].source")
        target = _text(edge.get("target"), f"graph {path}.links[{index}].target")
        relation = edge.get("kind", edge.get("relation"))
        relation = _text(relation, f"graph {path}.links[{index}].relation", identity=True)
        if source not in nodes or target not in nodes:
            raise AuditError(f"graph {path} edge {index} has a missing endpoint")
        facts[(source, target, relation)].append(edge)
    return _GraphIndex(
        nodes=nodes,
        facts={
            key: tuple(
                sorted(
                    edges,
                    key=lambda edge: json.dumps(edge, sort_keys=True, separators=(",", ":")),
                )
            )
            for key, edges in facts.items()
        },
    )


def _edge_anchor(edge: dict[str, Any]) -> tuple[str, int, int] | None:
    for raw in (
        edge.get("relationshipSite"),
        edge.get("source_anchor"),
        edge.get("sourceAnchor"),
    ):
        if not isinstance(raw, dict):
            continue
        file = raw.get("file", raw.get("source_file"))
        start = raw.get("startByte", raw.get("start_byte"))
        end = raw.get("endByte", raw.get("end_byte"))
        if (
            isinstance(file, str)
            and isinstance(start, int)
            and not isinstance(start, bool)
            and isinstance(end, int)
            and not isinstance(end, bool)
        ):
            return (file, start, end)
    return None


def _require_fact(
    graph: _GraphIndex,
    fact: AuditGraphFact,
    occurrence: AuditOccurrence,
    record_id: str,
) -> None:
    edges = graph.facts.get((fact.source, fact.target, fact.relation), ())
    expected_anchor = (occurrence.file, occurrence.start_byte, occurrence.end_byte)
    if not edges:
        raise AuditError(
            f"record {record_id!r} references an absent graph fact "
            f"{fact.source} -[{fact.relation}]-> {fact.target}"
        )
    if not any(_edge_anchor(edge) == expected_anchor for edge in edges):
        raise AuditError(
            f"record {record_id!r} has no graph occurrence at "
            f"{occurrence.file}:{occurrence.start_byte}-{occurrence.end_byte}"
        )


def _validate_record_inputs(
    manifest: AuditManifest,
    corpus_root: Path,
    graph_root: Path,
) -> dict[str, _GraphIndex]:
    indexes: dict[str, _GraphIndex] = {}
    corpus_roots: dict[str, Path] = {}
    single = len(manifest.corpora) == 1
    for corpus in manifest.corpora:
        root = corpus_root if single and corpus.path == "." else corpus_root / corpus.path
        if not root.is_dir():
            raise AuditError(f"corpus root does not exist: {root}")
        actual_commit = _corpus_commit(root)
        if actual_commit != corpus.commit:
            raise AuditError(
                f"corpus {corpus.name!r} commit mismatch: "
                f"expected {corpus.commit}, observed {actual_commit}"
            )
        graph_path = (
            graph_root
            if single and graph_root.is_file()
            else graph_root / corpus.graph
        )
        if not graph_path.is_file():
            raise AuditError(f"graph does not exist for corpus {corpus.name!r}: {graph_path}")
        actual_graph_sha256 = _file_sha256(graph_path)
        if actual_graph_sha256 != corpus.graph_sha256:
            raise AuditError(
                f"graph digest mismatch for corpus {corpus.name!r}: "
                f"expected {corpus.graph_sha256}, observed {actual_graph_sha256}"
            )
        indexes[corpus.name] = _index_graph(graph_path)
        corpus_roots[corpus.name] = root

    for record in manifest.records:
        root = corpus_roots[record.corpus]
        source_path = root / record.occurrence.file
        try:
            contents = source_path.read_bytes()
        except OSError as error:
            raise AuditError(
                f"record {record.record_id!r} source is unavailable: {error}"
            ) from error
        if record.occurrence.end_byte > len(contents):
            raise AuditError(
                f"record {record.record_id!r} occurrence exceeds "
                f"{record.occurrence.file}"
            )
        snippet = contents[
            record.occurrence.start_byte : record.occurrence.end_byte
        ].replace(b"\r\n", b"\n")
        actual_snippet_sha256 = hashlib.sha256(snippet).hexdigest()
        if actual_snippet_sha256 != record.occurrence.snippet_sha256:
            raise AuditError(
                f"record {record.record_id!r} has a stale snippet hash: "
                f"expected {record.occurrence.snippet_sha256}, "
                f"observed {actual_snippet_sha256}"
            )

        graph = indexes[record.corpus]
        if record.source.node_id not in graph.nodes:
            raise AuditError(
                f"record {record.record_id!r} source node is absent from the graph"
            )
        fact = AuditGraphFact(
            record.source.node_id,
            record.target.node_id,
            record.relation,
        )
        expected_anchor = (
            record.occurrence.file,
            record.occurrence.start_byte,
            record.occurrence.end_byte,
        )
        present = fact.target in graph.nodes and any(
            _edge_anchor(edge) == expected_anchor
            for edge in graph.facts.get(
                (fact.source, fact.target, fact.relation),
                (),
            )
        )
        if record.pool == "accepted":
            _require_fact(graph, fact, record.occurrence, record.record_id)
        elif record.judgment in {"correct", "external"}:
            _require_fact(graph, fact, record.occurrence, record.record_id)
        elif record.judgment == "represented_elsewhere":
            assert record.representation is not None
            _require_fact(
                graph,
                record.representation,
                record.occurrence,
                record.record_id,
            )
        elif record.pool == "source_oracle" and present:
            raise AuditError(
                f"record {record.record_id!r} is classified {record.judgment!r} "
                "but the graph fact exists"
            )
    return indexes


def _record_contribution(record: AuditRecord) -> tuple[int, int, int, int]:
    precision_denominator = int(record.pool == "accepted")
    precision_numerator = int(
        record.pool == "accepted" and record.judgment in CORRECT_ACCEPTED_JUDGMENTS
    )
    recall_denominator = int(
        record.pool == "source_oracle" and record.judgment in RECALL_TRUTH_JUDGMENTS
    )
    recall_numerator = int(
        record.pool == "source_oracle"
        and record.judgment in RECALL_RECOVERED_JUDGMENTS
    )
    return (
        precision_numerator,
        precision_denominator,
        recall_numerator,
        recall_denominator,
    )


def _strata(
    records: tuple[AuditRecord, ...],
) -> tuple[
    dict[str, dict[str, dict[str, int | float | None]]],
    dict[str, dict[str, list[AuditRecord]]],
]:
    dimensions = {
        "corpus": lambda record: record.corpus,
        "language": lambda record: record.language,
        "relation": lambda record: record.relation,
        "capability": lambda record: record.capability,
        "confidence": lambda record: record.confidence,
        "targetCluster": lambda record: record.target_cluster,
    }
    grouped: dict[str, dict[str, list[AuditRecord]]] = {}
    result: dict[str, dict[str, dict[str, int | float | None]]] = {}
    for dimension, key_for in dimensions.items():
        groups: dict[str, list[AuditRecord]] = defaultdict(list)
        for record in records:
            groups[key_for(record)].append(record)
        grouped[dimension] = dict(groups)
        result[dimension] = {}
        for key in sorted(groups):
            values = groups[key]
            contributions = [_record_contribution(record) for record in values]
            precision_numerator = sum(value[0] for value in contributions)
            precision_denominator = sum(value[1] for value in contributions)
            recall_numerator = sum(value[2] for value in contributions)
            recall_denominator = sum(value[3] for value in contributions)
            result[dimension][key] = {
                "records": len(values),
                "correctAccepted": precision_numerator,
                "auditedAccepted": precision_denominator,
                "precision": (
                    precision_numerator / precision_denominator
                    if precision_denominator
                    else None
                ),
                "recovered": recall_numerator,
                "recallCandidates": recall_denominator,
                "recall": (
                    recall_numerator / recall_denominator
                    if recall_denominator
                    else None
                ),
            }
    return result, grouped


def _qualification_failures(
    manifest: AuditManifest,
    records: tuple[AuditRecord, ...],
    strata: dict[str, dict[str, dict[str, int | float | None]]],
    grouped: dict[str, dict[str, list[AuditRecord]]],
    precision: AuditMetric,
) -> list[str]:
    failures: list[str] = []
    if precision.denominator < QUALIFICATION_MINIMUM:
        failures.append(
            f"accepted audit sample has {precision.denominator} records; "
            f"{QUALIFICATION_MINIMUM} required"
        )
    for corpus in manifest.corpora:
        count = int(strata["corpus"].get(corpus.name, {}).get("auditedAccepted", 0))
        if count < CORPUS_MINIMUM:
            failures.append(
                f"corpus {corpus.name!r} has {count} accepted records; "
                f"{CORPUS_MINIMUM} required"
            )
    for relation in manifest.required_relations:
        count = int(strata["relation"].get(relation, {}).get("auditedAccepted", 0))
        if count < RELATION_MINIMUM:
            failures.append(
                f"relation {relation!r} has {count} accepted records; "
                f"{RELATION_MINIMUM} required"
            )
    for identity in manifest.advertised_capabilities:
        values = [
            record
            for record in records
            if record.adapter == identity.adapter
            and record.framework_pack == identity.framework_pack
            and record.capability == identity.capability
        ]
        accepted = sum(record.pool == "accepted" for record in values)
        if accepted < CAPABILITY_MINIMUM:
            failures.append(
                f"capability {(identity.adapter, identity.framework_pack, identity.capability)!r} "
                f"has {accepted} accepted records; {CAPABILITY_MINIMUM} required"
            )
        correct = sum(
            record.pool == "accepted"
            and record.judgment in CORRECT_ACCEPTED_JUDGMENTS
            for record in values
        )
        capability_precision = correct / accepted if accepted else 0.0
        if accepted and capability_precision < CAPABILITY_PRECISION_GATE:
            failures.append(
                f"capability {(identity.adapter, identity.framework_pack, identity.capability)!r} "
                f"precision {capability_precision:.6f} is below "
                f"{CAPABILITY_PRECISION_GATE:.3f}"
            )
        recall_denominator = sum(
            record.pool == "source_oracle"
            and record.judgment in RECALL_TRUTH_JUDGMENTS
            for record in values
        )
        recall_numerator = sum(
            record.pool == "source_oracle"
            and record.judgment in RECALL_RECOVERED_JUDGMENTS
            for record in values
        )
        capability_recall = (
            recall_numerator / recall_denominator if recall_denominator else 0.0
        )
        if recall_denominator == 0:
            failures.append(
                f"capability {(identity.adapter, identity.framework_pack, identity.capability)!r} "
                "has no source-derived recall candidates"
            )
        elif capability_recall < CAPABILITY_RECALL_GATE:
            failures.append(
                f"capability {(identity.adapter, identity.framework_pack, identity.capability)!r} "
                f"recall {capability_recall:.6f} is below "
                f"{CAPABILITY_RECALL_GATE:.3f}"
            )
    if precision.observed is None or precision.observed < PRECISION_GATE:
        failures.append(
            f"overall precision {precision.observed!r} is below {PRECISION_GATE:.3f}"
        )
    if (
        precision.wilson_95 is None
        or precision.wilson_95.lower < PRECISION_WILSON_LOWER_GATE
    ):
        lower = None if precision.wilson_95 is None else precision.wilson_95.lower
        failures.append(
            f"Wilson 95% precision lower bound {lower!r} is below "
            f"{PRECISION_WILSON_LOWER_GATE:.3f}"
        )
    for dimension in ("corpus", "language", "relation", "capability"):
        for key, values in sorted(grouped[dimension].items()):
            accepted = [record for record in values if record.pool == "accepted"]
            if not accepted:
                continue
            clusters = Counter(record.target_cluster for record in accepted)
            cluster, count = max(clusters.items(), key=lambda item: (item[1], item[0]))
            if count / len(accepted) > TARGET_CLUSTER_MAXIMUM_FRACTION:
                failures.append(
                    f"target cluster {cluster!r} supplies {count}/{len(accepted)} "
                    f"accepted records in {dimension} stratum {key!r}; "
                    "maximum is 10%"
                )
    return failures


def run_audit(manifest_path: Path, graph_path: Path, corpus_path: Path) -> AuditResult:
    manifest = load_manifest(manifest_path)
    _validate_record_inputs(manifest, corpus_path, graph_path)
    manifest_digest = hashlib.sha256(
        json.dumps(
            json.loads(manifest_path.read_text(encoding="utf-8")),
            sort_keys=True,
            separators=(",", ":"),
            ensure_ascii=False,
        ).encode("utf-8")
    ).hexdigest()
    records = tuple(sorted(manifest.records, key=lambda record: record.record_id))
    contributions = [_record_contribution(record) for record in records]
    precision = _metric(
        sum(value[0] for value in contributions),
        sum(value[1] for value in contributions),
        interval=True,
    )
    recall = _metric(
        sum(value[2] for value in contributions),
        sum(value[3] for value in contributions),
    )
    judgments = Counter(record.judgment for record in records)
    critical = {
        judgment: judgments.get(judgment, 0) for judgment in CRITICAL_JUDGMENTS
    }
    strata, grouped = _strata(records)
    failures = [
        f"critical semantic violation {judgment!r}: {count}"
        for judgment, count in critical.items()
        if count
    ]
    if manifest.mode == "qualification":
        failures.extend(
            _qualification_failures(
                manifest,
                records,
                strata,
                grouped,
                precision,
            )
        )
    failures = sorted(set(failures))
    eligible = manifest.mode == "qualification" and not failures
    return AuditResult(
        schema=AUDIT_RESULT_SCHEMA,
        mode=manifest.mode,
        passed=not failures,
        eligible_for_quality_claim=eligible,
        manifest_sha256=manifest_digest,
        audited_records=len(records),
        audited_accepted_edges=precision.denominator,
        precision=precision,
        recall=recall,
        judgments={key: judgments[key] for key in sorted(judgments)},
        critical_violations=critical,
        strata=strata,
        failures=tuple(failures),
    )


def audit_result_json_value(result: AuditResult) -> dict[str, Any]:
    """Serialize an audit result with the public camelCase field contract."""

    public_keys = {
        "eligible_for_quality_claim": "eligibleForQualityClaim",
        "manifest_sha256": "manifestSha256",
        "audited_records": "auditedRecords",
        "audited_accepted_edges": "auditedAcceptedEdges",
        "wilson_95": "wilson95",
        "critical_violations": "criticalViolations",
    }

    def convert(value: Any) -> Any:
        if isinstance(value, dict):
            return {
                public_keys.get(key, key): convert(item)
                for key, item in value.items()
            }
        if isinstance(value, list):
            return [convert(item) for item in value]
        return value

    converted = convert(to_json_value(result))
    assert isinstance(converted, dict)
    return converted
