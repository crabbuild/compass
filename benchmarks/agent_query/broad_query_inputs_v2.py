"""Bound retained native inputs to their original frozen files for protocol2.

This is supplied-byte verification, not execution or release authentication.
No product, graph query, installer or network client is invoked here.
"""
from pathlib import Path

from benchmarks.agent_query import broad_query_capture as capture
from benchmarks.agent_query import broad_query_review as old
from benchmarks.agent_query.broad_query_lineage_v2 import _reference, require_schema

MAX_FILES = 65536
MAX_BYTES = 2 * 1024**3
MAX_DEP_FILES = 65536
MAX_DEP_BYTES = 512 * 1024**2


class BoundedArtifacts(old.Artifacts):
    """Version2 precharges unique artifacts before reading/hashing payloads."""
    def __init__(self, root):
        super().__init__(root)
        self.sizes = {}

    def read(self, reference, maximum=old.MAX_JSON_BYTES):
        _reference(reference)
        path, digest = reference["path"], reference["sha256"]
        previous = self.seen.get(path)
        if previous is not None and previous != digest:
            raise ValueError("conflicting retained artifact identity")
        if previous is None and len(self.seen) >= MAX_FILES:
            raise ValueError("retained artifact count bound exceeded before read")
        allowance = self.sizes[path] if previous is not None else MAX_BYTES - self.total
        data = super().read(reference, min(maximum, allowance))
        self.sizes[path] = len(data)
        return data

    def recheck(self):
        for path in sorted(self.absent_source_queries):
            old.require_absent_source_query(path)
        for path in sorted(self.regular_source_queries):
            old.require_regular_source_query(path)
        for path, digest in sorted(self.seen.items()):
            self.read(dict(path=path, sha256=digest), self.sizes[path])


class FrozenInputs:
    def __init__(self):
        self.seen, self.total = {}, 0
        self.absent, self.regular = set(), set()

    def read(self, reference, maximum=old.MAX_JSON_BYTES):
        if type(reference) is not dict or reference.keys() != {"path", "sha256"}:
            raise ValueError("original input reference needs exactly path and sha256")
        path = Path(old._text(reference["path"], "absolute original input path"))
        if not path.is_absolute():
            raise ValueError("original input paths must be absolute")
        digest = old._digest(reference["sha256"])
        previous = self.seen.get(path)
        if previous is not None and previous[0] != digest:
            raise ValueError("conflicting original input identity")
        if previous is None and len(self.seen) >= MAX_FILES:
            raise ValueError("original input count bound exceeded before read")
        allowance = previous[1] if previous is not None else MAX_BYTES - self.total
        _, data = capture.external(reference, min(maximum, allowance))
        if previous is None:
            self.total += len(data)
        self.seen[path] = (digest, len(data))
        return data

    def recheck(self):
        for path in sorted(self.absent):
            old.require_absent_source_query(path)
        for path in sorted(self.regular):
            old.require_regular_source_query(path)
        for path, (digest, maximum) in sorted(self.seen.items()):
            capture.external(dict(path=str(path), sha256=digest), maximum)


def verify_native_inputs(manifest, registration, run, artifacts):
    """Verify the complete source-frozen native inventory before row scoring.

    dependencies is an ordered per-tool list of {original, artifact} records
    matching every file in its frozen environment inventory. Both originals
    and retained bytes remain required; final recheck uses the original paths,
    including declared executable symlinks rather than only their old targets.
    """
    originals = FrozenInputs()
    spec = require_schema(artifacts.document(_reference(run.get("captureInput"))), "capture-input")
    invocation = run.get("captureInputInvocation")
    if invocation is not None:
        if type(invocation) is not dict or invocation.keys() != {"original", "artifact"}:
            raise ValueError("capture-input invocation requires original and artifact references")
        data = originals.read(invocation["original"])
        reference = _reference(invocation["artifact"])
        if (reference["sha256"] != invocation["original"]["sha256"]
                or capture.encoded(old._json(data)) != capture.encoded(spec)):
            raise ValueError("capture-input invocation differs from retained validated input")
        artifacts.read(reference, len(data))
    if not spec.keys() <= {"schema", "registration", "plan", "sourceRoots", "exposureEvidence", "calibrationEvidence", "panels"}:
        raise ValueError("unsupported original capture input fields")
    if spec.get("plan") != registration["capturePlan"]:
        raise ValueError("original capture plan differs from source registration")
    from benchmarks.agent_query.broad_query_panels_v2 import read_panels, ROLES
    actors = registration["capturePlan"]["actorSha256"]
    original_panels = read_panels(registration, spec.get("panels"), originals.read, actors, external=True)
    retained_panels = read_panels(registration, run.get("panels"), artifacts.read, actors)
    if (original_panels is None) != (retained_panels is None):
        raise ValueError("original/retained panel presence differs")
    if original_panels is not None:
        if (original_panels["definition"] != retained_panels["definition"]
                or original_panels["registrations"] != retained_panels["registrations"]):
            raise ValueError("original/retained panel definitions differ")
        for role in ROLES:
            old.verify_calibration_evidence(retained_panels["registrations"][role],
                manifest.get("calibrationEvidence", {}), artifacts.read)
    reference = spec.get("registration")
    originals.read(reference)
    if reference["sha256"] != manifest["registration"]["sha256"]:
        raise ValueError("original source registration differs from retained registration")
    repositories = {repo["id"] for repo in registration["repositories"]}
    for roots in (spec.get("sourceRoots"), manifest.get("sourceRoots")):
        if type(roots) is not dict or roots.keys() != repositories:
            raise ValueError("source roots must match the entire frozen repository matrix")
        for value in roots.values():
            if not Path(old._text(value, "absolute source root")).is_absolute():
                raise ValueError("source roots must be absolute")
    if any(Path(spec["sourceRoots"][key]).resolve() != Path(manifest["sourceRoots"][key]).resolve()
           for key in repositories):
        raise ValueError("source roots differ from original capture")
    for name in ("exposureEvidence", "calibrationEvidence"):
        external_refs = old._object(spec.get(name, {}), name)
        retained_refs = old._object(manifest.get(name, {}), name)
        if external_refs.keys() != retained_refs.keys():
            raise ValueError("original and retained history evidence partitions differ")
        for digest, ref in external_refs.items():
            originals.read(ref)
            if ref["sha256"] != digest or retained_refs[digest].get("sha256") != digest:
                raise ValueError("history evidence differs from its frozen digest")
            artifacts.read(_reference(retained_refs[digest]))
    plan = spec["plan"]
    originals.read(plan.get("environmentExecutable"), old.MAX_GRAPH_BYTES)
    originals.read(plan.get("measurement", {}).get("executable"), old.MAX_GRAPH_BYTES)
    frozen_tools = old._array(plan.get("tools"), "frozen tools", 2, 2)
    actual_tools = old._array(run.get("tools"), "retained tools", 2, 2)
    for tool in (*frozen_tools, *actual_tools):
        old._object(tool, "native tool record")
    if ({old._choice(tool.get("id"), old.TOOLS, "tool") for tool in frozen_tools} != set(old.TOOLS)
            or {old._choice(tool.get("id"), old.TOOLS, "tool") for tool in actual_tools} != set(old.TOOLS)):
        raise ValueError("native tool matrix is missing or duplicated")
    dependencies, dependency_bytes = {}, 0
    for frozen in frozen_tools:
        actual = next(tool for tool in actual_tools if tool["id"] == frozen["id"])
        if ("releaseVerification" in frozen) != ("releaseVerification" in actual):
            raise ValueError("release evidence presence differs from the source-frozen declaration")
        originals.read(frozen.get("executable"), old.MAX_GRAPH_BYTES)
        environment = old._json(originals.read(frozen.get("environment")))
        files = old._array(environment.get("files"), "frozen dependency files", 65536, 1)
        retained = old._array(actual.get("dependencies"), "retained dependency files", 65536, 1)
        if len(retained) != len(files):
            raise ValueError("retained dependency partition differs from original environment")
        for original, item in zip(files, retained):
            if (type(item) is not dict or item.keys() != {"original", "artifact"}
                    or item["original"] != original):
                raise ValueError("retained dependencies differ from the exact original ordered inventory")
            path = old._text(old._object(original, "original dependency").get("path"), "dependency path")
            known = path in dependencies
            if not known and len(dependencies) >= MAX_DEP_FILES:
                raise ValueError("dependency count bound exceeded before read")
            allowance = dependencies[path] if known else MAX_DEP_BYTES - dependency_bytes
            data = originals.read(original, min(old.MAX_GRAPH_BYTES, allowance))
            retained_ref = _reference(item["artifact"])
            if retained_ref["sha256"] != original["sha256"]:
                raise ValueError("retained dependency bytes differ from original")
            if not known:
                dependencies[path] = len(data)
                dependency_bytes += len(data)
            artifacts.read(retained_ref, len(data))
        if "releaseVerification" in frozen:
            originals.read(frozen["releaseVerification"])
            actual_ref = _reference(actual.get("releaseVerification"))
            artifacts.read(actual_ref)
            if actual_ref["sha256"] != frozen["releaseVerification"]["sha256"]:
                raise ValueError("retained release evidence differs from the pre-output freeze")
    graphs = old._array(plan.get("graphs"), "source-frozen graphs", 64, 2)
    expected = {(repo, tool) for repo in repositories for tool in old.TOOLS}
    for entries in (graphs, old._array(run.get("graphs"), "retained graphs", 64, 2)):
        keys = []
        for entry in entries:
            old._object(entry, "graph input record")
            keys.append((old._choice(entry.get("repository"), tuple(repositories), "graph repository"),
                old._choice(entry.get("tool"), old.TOOLS, "graph tool")))
        if len(keys) != len(expected) or set(keys) != expected:
            raise ValueError("graph matrix is missing or duplicated")
    for entry in graphs:
        originals.read(entry.get("artifact"), old.MAX_GRAPH_BYTES)
        originals.read(entry.get("sourceInventory"))
        build = old._json(originals.read(entry.get("buildCapture")))
        if not old.verify_build_capture(build, originals.read):
            raise ValueError("original graph build was not a successful bounded capture")
        source_ref = old.source_query_reference(entry)
        if entry["tool"] == "compass":
            sibling = Path(entry["artifact"]["path"]).with_name(old.SOURCE_QUERY_FILE)
            if source_ref is None:
                old.require_absent_source_query(sibling)
                originals.absent.add(sibling)
            else:
                old.require_regular_source_query(sibling)
                originals.regular.add(sibling)
                old.validate_source_query(originals.read(source_ref, old.MAX_SOURCE_QUERY_BYTES), entry["artifact"]["sha256"])
    return originals
