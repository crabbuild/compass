"""Artifact-bound graph-coverage diagnostics; never independent qualification.

This checks recorded bytes and source anchors. Review truth, edge-to-source
semantics, runtime authenticity, precision and prospective power remain separate.
No product, graph producer, model service or competitor installation is invoked.
"""
from collections import Counter
from pathlib import Path
import json
import re
import stat

from benchmarks.agent_query import broad_graph_statistics as statistics
from benchmarks.agent_query import broad_query_review as query
from benchmarks.agent_query import community_tasks


INPUT_SCHEMA = "compass.broad-graph-coverage-input/1"
REGISTRATION_SCHEMA = "compass.broad-graph-coverage-registration/1"
CAPTURE_SCHEMA = "compass.broad-graph-coverage-capture/1"
REVIEW_SCHEMA = "compass.broad-graph-coverage-reviews/1"
RESULT_SCHEMA = "compass.broad-graph-coverage-evidence/1"
MAX_SOURCE_FILES = 65_536
MAX_SOURCE_BYTES = 256 * 1024 * 1024
MAX_RECORD_PROOFS = 64
MAX_RECORD_PROOFS_TOTAL = 262_144
MAX_RECORD_BYTES_TOTAL = 128 * 1024 * 1024
STATUSES = ("present", "missing", "ambiguous", "unavailable_contract", "unreviewed")


def _canonical_bytes(value):
    return json.dumps(value, sort_keys=True, ensure_ascii=False,
        allow_nan=False, separators=(",", ":")).encode()


def canonical_sha256(value):
    return query.sha(_canonical_bytes(value))


def support_hashes():
    return {name: query.sha(community_tasks.read_bounded(Path(path), query.MAX_JSON_BYTES))
        for name, path in [("evidence", __file__), ("statistics", statistics.__file__),
                           ("query", query.__file__), ("reader", community_tasks.__file__)]}


def _fields(value, expected, where):
    value = query._object(value, where)
    if set(value) != set(expected):
        raise ValueError(f"{where} fields differ from the version1 contract")
    return value


def _regular(path):
    if not stat.S_ISREG(path.stat().st_mode):
        raise ValueError("graph evidence inputs must be regular files")


class _Artifacts(query.Artifacts):
    def read(self, ref, maximum=query.MAX_JSON_BYTES):
        ref = query._object(ref, "graph artifact reference")
        path = (self.root / query._relative(ref.get("path"))).resolve()
        if not path.is_relative_to(self.root):
            raise ValueError("graph artifact escapes evidence root")
        _regular(path)
        return super().read(ref, maximum)

    def recheck(self):
        # Repeat containment/type checks as well as hashes after assessment.
        for path, digest in sorted(self.seen.items()):
            self.read(dict(path=path, sha256=digest), query.MAX_GRAPH_BYTES)


def _tool_identity(tool, artifacts):
    _fields(tool, ("id", "version", "commit", "profile", "executable", "environment", "configuration"), "graph tool")
    query._choice(tool["id"], statistics.TOOLS, "graph tool ID")
    for field in ("version", "commit", "profile"):
        query._text(tool[field], f"graph tool {field}", 128)
    for field in ("executable", "environment", "configuration"):
        artifacts.read(tool[field], query.MAX_GRAPH_BYTES if field == "executable" else query.MAX_JSON_BYTES)
    return {field: tool[field] for field in ("id", "version", "commit", "profile")} | {
        field: tool[field]["sha256"] for field in ("executable", "environment", "configuration")}


def _span(span, files, *, declaration=False):
    expected = ("file", "startByte", "endByte", "sha256")
    _fields(span, (*expected, "symbol", "owner") if declaration else expected, "source anchor")
    file = query._relative(span["file"])
    if file not in files:
        raise ValueError("source anchor is outside the frozen file inventory")
    data = files[file]
    start = query._integer(span["startByte"], "source start byte", 0, len(data))
    end = query._integer(span["endByte"], "source end byte", start + 1, len(data))
    if query.sha(data[start:end]) != query._digest(span["sha256"], "source anchor SHA-256"):
        raise ValueError("source anchor digest differs from pinned source bytes")
    if declaration:
        query._text(span["symbol"], "source symbol", 512)
        query._text(span["owner"], "source owner", 1024, empty=True)
    return file, start, end


def _sources(repositories, roots):
    if set(query._object(roots, "source roots")) != {r["id"] for r in repositories}:
        raise ValueError("source-root matrix differs from registration")
    contents, inventory, identities = {}, {}, {}
    total, count = 0, 0
    for repo in repositories:
        _fields(repo, ("id", "commit", "files"), "source repository")
        name = query._text(repo["id"], "repository ID", 128)
        if name in contents:
            raise ValueError("duplicate source repository")
        if not re.fullmatch(r"[0-9a-f]{40}", query._text(repo["commit"], "source commit", 40)):
            raise ValueError("source commit must be an exact Git object ID")
        root = Path(query._text(roots[name], "source root", 4096)).resolve()
        files = {}
        for item in query._array(repo["files"], "source file inventory", MAX_SOURCE_FILES, 1):
            _fields(item, ("file", "sha256"), "source file")
            file = query._relative(item["file"])
            if file in files:
                raise ValueError("duplicate source inventory file")
            path = (root / file).resolve()
            if not path.is_relative_to(root):
                raise ValueError("source file escapes registered root")
            _regular(path)
            data = community_tasks.read_bounded(path, query.MAX_SOURCE_BYTES)
            total += len(data)
            count += 1
            if total > MAX_SOURCE_BYTES or count > MAX_SOURCE_FILES:
                raise ValueError("aggregate graph source inventory limit exceeded")
            if query.sha(data) != query._digest(item["sha256"]):
                raise ValueError("graph source file digest mismatch")
            files[file] = data
            identities[name, file] = (root, file, item["sha256"])
        contents[name] = files
        inventory[name] = repo
    return contents, inventory, identities


def assess_graph_coverage(manifest, root):
    """Bind paired manual observations without turning attestations into truth.

    A positive observation needs an exact raw edge-record digest in its pinned
    graph. That proves a referenced record exists, not that its relationship,
    owner, direction or occurrence agrees with source. This deliberately remains
    diagnostic until the independent semantic/precision and joint gates exist.
    """
    _fields(manifest, ("schema", "registration", "capture", "reviews", "sourceRoots"), "graph coverage input")
    if manifest["schema"] != INPUT_SCHEMA:
        raise ValueError("unsupported graph coverage input schema")
    artifacts = _Artifacts(Path(root))
    registration = artifacts.document(manifest["registration"])
    _fields(registration, ("schema", "frozenAt", "supportSha256", "repositories", "tools", "facts", "resamples", "seed"), "graph registration")
    if registration["schema"] != REGISTRATION_SCHEMA or registration["supportSha256"] != support_hashes():
        raise ValueError("unsupported or changed graph coverage analysis protocol")
    frozen = query._timestamp(registration["frozenAt"])
    repositories = query._array(registration["repositories"], "graph repositories", statistics.MAX_REPOSITORIES, 1)
    sources, repository_map, source_paths = _sources(repositories, manifest["sourceRoots"])
    tool_map = {}
    for tool in query._array(registration["tools"], "frozen graph tools", 2, 2):
        identity = _tool_identity(tool, artifacts)
        if identity["id"] in tool_map:
            raise ValueError("duplicate frozen graph tool")
        tool_map[identity["id"]] = identity
    if set(tool_map) != set(statistics.TOOLS):
        raise ValueError("both graph tools must be frozen")
    facts, anchors = {}, set()
    for item in query._array(registration["facts"], "frozen source facts", statistics.MAX_SOURCE_FACTS, 1):
        _fields(item, (*statistics.TEXT_FIELDS, "fact"), "frozen source fact")
        for field in statistics.TEXT_FIELDS:
            query._text(item[field], f"source fact {field}", 128)
        repo = query._choice(item["repository"], repository_map, "source fact repository")
        fact = _fields(item["fact"], ("source", "target", "occurrence", "requiresDirection"), "source relationship")
        query._boolean(fact["requiresDirection"], "required direction contract")
        positions = tuple(_span(fact[field], sources[repo], declaration=field != "occurrence")
            for field in ("source", "target", "occurrence"))
        key = (repo, item["id"])
        # Changing labels/owners/IDs cannot manufacture a second physical unit.
        physical = (repo, item["relation"], positions)
        if key in facts or physical in anchors:
            raise ValueError("duplicate graph source fact or physical relationship occurrence")
        anchors.add(physical)
        digest = canonical_sha256(dict(relation=item["relation"], **fact))
        facts[key] = dict(item, sourceFactSha256=digest)
    capture = artifacts.document(manifest["capture"])
    _fields(capture, ("schema", "registrationSha256", "startedAt", "tools", "graphs"), "graph capture")
    if capture["schema"] != CAPTURE_SCHEMA or capture["registrationSha256"] != manifest["registration"]["sha256"]:
        raise ValueError("graph capture is not bound to the frozen registration")
    started = query._timestamp(capture["startedAt"])
    if frozen >= started:
        raise ValueError("graph registration must precede collection")
    captured_tools = [_tool_identity(t, artifacts) for t in query._array(capture["tools"], "captured graph tools", 2, 2)]
    if sorted(captured_tools, key=lambda t: t["id"]) != sorted(tool_map.values(), key=lambda t: t["id"]):
        raise ValueError("captured tool bytes/profile differ from frozen identities")
    graphs, build_failures = {}, []
    for entry in query._array(capture["graphs"], "captured graph matrix", 64, 2):
        entry = query._object(entry, "captured graph")
        key = (entry.get("repository"), entry.get("tool"))
        if key in graphs or key[0] not in repository_map or key[1] not in tool_map:
            raise ValueError("duplicate or unexpected captured graph")
        repo = repository_map[key[0]]
        if entry.get("sourceCommit") != repo["commit"] or entry.get("sourceCleanBeforeAndAfter") is not True:
            raise ValueError("graph source pin or before/after clean attestation differs")
        if artifacts.document(entry.get("sourceInventory")).get("files") != repo["files"]:
            raise ValueError("graph build inventory differs from source registration")
        build = artifacts.document(entry.get("buildCapture"))
        build_ok = query.verify_build_capture(build, artifacts.read)
        if not build_ok:
            build_failures.append(dict(repository=key[0], tool=key[1],
                exitCode=build["exit_code"], timedOut=build["timed_out"],
                outputLimited=build["output_limited"], wallMs=build["wall_ms"]))
        graph = artifacts.document(entry.get("artifact"), query.MAX_GRAPH_BYTES)
        # Existing bounded graph validation checks IDs, endpoints and metadata;
        # its name/site lookup is intentionally not used as a semantic oracle.
        query.GraphIndex(graph, key[1])
        graphs[key] = (entry, graph, build_ok)
    if set(graphs) != {(repo, tool) for repo in repository_map for tool in statistics.TOOLS}:
        raise ValueError("captured graph matrix is incomplete")
    reviews = artifacts.document(manifest["reviews"])
    _fields(reviews, ("schema", "captureSha256", "reviewedAt", "observations"), "graph reviews")
    if reviews["schema"] != REVIEW_SCHEMA or reviews["captureSha256"] != manifest["capture"]["sha256"]:
        raise ValueError("graph reviews differ from the captured graph set")
    if query._timestamp(reviews["reviewedAt"]) < started:
        raise ValueError("graph review predates collection")
    observed, record_hashes = {}, {}
    proof_count, record_bytes = 0, 0
    for item in query._array(reviews["observations"], "graph observations", 2 * statistics.MAX_SOURCE_FACTS):
        _fields(item, ("repository", "factId", "tool", "sourceFactSha256", "graphSha256", "status", "reason", "edges"), "graph observation")
        key = (item["repository"], item["factId"], item["tool"])
        fact_key, graph_key = key[:2], (key[0], key[2])
        if key in observed or fact_key not in facts or graph_key not in graphs:
            raise ValueError("duplicate or unexpected graph observation")
        fact = facts[fact_key]
        entry, graph, build_ok = graphs[graph_key]
        if item["sourceFactSha256"] != fact["sourceFactSha256"] or item["graphSha256"] != entry["artifact"]["sha256"]:
            raise ValueError("graph observation source-fact or graph digest differs")
        status = query._choice(item["status"], STATUSES, "graph observation status")
        query._text(item["reason"], "graph observation rationale")
        edges = query._array(item["edges"], "graph record proofs", MAX_RECORD_PROOFS)
        proof_count += len(edges)
        if proof_count > MAX_RECORD_PROOFS_TOTAL:
            raise ValueError("aggregate graph record proof limit exceeded")
        if status != "present" and edges:
            raise ValueError("non-present graph observation cannot assert supporting edges")
        if status == "present" and (not edges or not build_ok):
            raise ValueError("present graph observation requires bounded successful build and record proof")
        if status == "present" and fact["fact"]["requiresDirection"] and not graph["directed"]:
            raise ValueError("undirected graph cannot certify a directed contract; report unavailable_contract")
        seen = set()
        records = graph.get("edges", graph.get("links", []))
        for proof in edges:
            _fields(proof, ("index", "sha256"), "raw graph record proof")
            index = query._integer(proof["index"], "raw edge index", 0, len(records) - 1)
            record_key = (*graph_key, index)
            if record_key not in record_hashes:
                data = _canonical_bytes(records[index])
                record_bytes += len(data)
                if record_bytes > MAX_RECORD_BYTES_TOTAL:
                    raise ValueError("aggregate canonical graph record byte limit exceeded")
                record_hashes[record_key] = query.sha(data)
            if index in seen or record_hashes[record_key] != query._digest(proof["sha256"]):
                raise ValueError("raw graph record proof is duplicated or differs from captured bytes")
            seen.add(index)
        observed[key] = (status, build_ok)
    rows, statuses = [], {tool: Counter() for tool in statistics.TOOLS}
    review_statuses = {tool: Counter() for tool in statistics.TOOLS}
    for (repo, identifier), fact in sorted(facts.items()):
        row = {field: fact[field] for field in statistics.TEXT_FIELDS}
        row["sourceFactSha256"] = fact["sourceFactSha256"]
        for tool in statistics.TOOLS:
            status, build_ok = observed.get((repo, identifier, tool), ("unreviewed", graphs[repo, tool][2]))
            review_statuses[tool][status] += 1
            statuses[tool][status if build_ok else "build_failure"] += 1
            # An unsuccessful build cannot establish graph quality, including
            # absence. Preserve the capture and withhold arithmetic for it.
            row[tool] = None if not build_ok or status == "unreviewed" else status == "present"
        rows.append(row)
    result = statistics.paired_graph_coverage(rows, resamples=registration["resamples"], seed=registration["seed"])
    for source_root, file, digest in source_paths.values():
        path = (source_root / file).resolve()
        if source_root.resolve() != source_root or not path.is_relative_to(source_root):
            raise ValueError("source file escapes registered root during recheck")
        _regular(path)
        if query.sha(community_tasks.read_bounded(path, query.MAX_SOURCE_BYTES)) != digest:
            raise ValueError("source file changed during graph assessment")
    artifacts.recheck()
    if registration["supportSha256"] != support_hashes():
        raise ValueError("analysis support files changed during graph assessment")
    return dict(schema=RESULT_SCHEMA, artifactBindingsVerified=True, sourceByteAnchorsVerified=True,
        qualificationEstablished=False, statistics=result,
        statusCounts={tool: dict(sorted(counts.items())) for tool, counts in statuses.items()},
        reviewStatusCounts={tool: dict(sorted(counts.items())) for tool, counts in review_statuses.items()},
        buildFailures=sorted(build_failures, key=lambda item: (item["repository"], item["tool"])),
        artifacts=dict(sorted(artifacts.seen.items())),
        limitations=["Manual source/edge semantics and review independence are not authenticated.",
            "A raw record reference does not prove owner, target, occurrence or provenance agreement.",
            "Failed builds remain unknown here; end-to-end failure/cost accounting is still required.",
            "Emitted-edge precision, production floors, prospective power and joint eligibility remain unimplemented."])
