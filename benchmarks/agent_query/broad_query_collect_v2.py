"""Bounded protocol2 collection for the explicit developer entry point.

Only explicit preinstalled tools/prebuilt graphs are queried. Synthetic tests
inject a runner; no collector invocation establishes independent source truth,
reviewer authentication, release authenticity or comparative qualification.
"""
from datetime import datetime, timezone
from pathlib import Path
import platform

from benchmarks.agent_query import broad_query_capture as capture
from benchmarks.agent_query import broad_query_review as old
from benchmarks.agent_query import runner as process
from benchmarks.agent_query.broad_query_actors_v2 import (
    ROLES, freeze_protocol, retain_protocol, verify_original_protocol,
)
from benchmarks.agent_query.broad_query_report_v2 import score_file_v2
from benchmarks.agent_query.broad_query_execution_v2 import parse_rss_v2
from benchmarks.agent_query.broad_query_inputs_v2 import (
    BoundedArtifacts, FrozenInputs, MAX_BYTES, MAX_FILES, MAX_DEP_BYTES, MAX_DEP_FILES,
)
from benchmarks.agent_query.broad_query_lineage_v2 import SCHEMAS, require_schema
from benchmarks.agent_query.broad_query_normalize_v2 import normalize_v2
from benchmarks.agent_query.broad_query_observation_v2 import _pending_judgment
from benchmarks.agent_query.broad_query_protocol import BundleWork, pack_claim_batches
from benchmarks.agent_query.broad_query_panels_v2 import read_panels, retain_panels


class CaptureBundle(capture.Bundle):
    """Preflight publication bounds and recheck every retained file, including raw.

    Raw subprocess streams reserve their maximum capacity before launch and are
    adopted without rewriting. Aggregate bytes count actual retained bytes;
    reservation can reject a run with insufficient remaining worst-case space.
    """
    def __init__(self, root, originals):
        super().__init__(root)
        self.originals, self.references = originals, {}
        self.graphs = []

    def _path(self, relative):
        relative = old._relative(relative)
        path = self.root / relative
        if relative in self.references or path.exists() or path.is_symlink():
            raise FileExistsError("capture publication cannot replace an existing artifact")
        if not path.resolve().is_relative_to(self.root):
            raise ValueError("capture publication path escapes its root")
        return path

    def _capacity(self, count, size):
        if len(self.references) + count > MAX_FILES:
            raise ValueError("capture artifact count bound exceeded before publication")
        if self.total + size > MAX_BYTES:
            raise ValueError("capture artifact byte bound exceeded before publication")

    def _register(self, relative, data):
        self._capacity(1, len(data))
        reference = dict(path=relative, sha256=old.sha(data))
        self.references[relative] = (reference, len(data))
        self.total += len(data)
        self.paths = len(self.references)
        return reference

    def put(self, relative, value):
        data = value if isinstance(value, bytes) else capture.encoded(value)
        path = self._path(relative)
        self._capacity(1, len(data))
        path.parent.mkdir(parents=True, exist_ok=True)
        capture.publish_exclusive(path, data)
        return self._register(relative, data)

    def copy(self, relative, reference, maximum=old.MAX_JSON_BYTES):
        self._path(relative)
        self._capacity(1, 0)
        data = self.originals.read(reference, min(maximum, MAX_BYTES - self.total))
        return self.put(relative, data)

    def execute(self, argv, relative, cwd, seconds, runner):
        path = self._path(relative)
        streams = [path.with_suffix("." + name) for name in ("stdout", "stderr")]
        for stream in streams:
            self._path(stream.relative_to(self.root).as_posix())
        # The shared runner writes at most MAX_OUTPUT_BYTES to each stream.
        # Reserve sidecar capacity too; failed reservation executes nothing.
        self._capacity(3, 2 * process.MAX_OUTPUT_BYTES + old.MAX_JSON_BYTES)
        path.parent.mkdir(parents=True, exist_ok=True)
        result = runner(argv, cwd=cwd, timeout_seconds=seconds,
            stdout_path=streams[0], stderr_path=streams[1])
        if tuple(result.argv) != tuple(argv):
            raise ValueError("runner returned a different command")
        refs, raw = {}, {}
        for name, stream in zip(("stdout", "stderr"), streams):
            if stream.is_symlink() or not stream.is_file() or stream.resolve() != stream:
                raise ValueError("raw capture must be a regular contained file")
            data = old.read_bounded(stream, process.MAX_OUTPUT_BYTES)
            raw[name] = data
            refs[name] = self._register(stream.relative_to(self.root).as_posix(), data)
        for name in ("timed_out", "output_limited"):
            old._boolean(getattr(result, name), "runner " + name)
        old._integer(result.wall_ms, "runner latency", 0, 86_400_000)
        if type(result.exit_code) is not int:
            raise ValueError("runner exit code must be an integer")
        record = dict(argv=list(argv), exit_code=result.exit_code, timed_out=result.timed_out,
            output_limited=result.output_limited, wall_ms=result.wall_ms,
            stdout_bytes=len(raw["stdout"]), stderr_bytes=len(raw["stderr"]), **refs)
        return self.put(relative, record), record, raw

    def recheck(self):
        self.originals.recheck()
        retained = BoundedArtifacts(self.root)
        for entry in self.graphs:
            retained.source_query(entry)
        for reference, size in self.references.values():
            retained.read(reference, size)
        retained.recheck()


def _preflight(spec, destination, input_reference=None):
    require_schema(spec, "capture-input")
    if not spec.keys() <= {"schema", "registration", "plan", "sourceRoots", "exposureEvidence", "calibrationEvidence", "panels"}:
        raise ValueError("unsupported capture-input fields")
    originals = FrozenInputs()
    if input_reference is not None:
        if capture.encoded(old._json(originals.read(input_reference))) != capture.encoded(spec):
            raise ValueError("capture-input invocation differs from supplied snapshot")
    registration = old.validate_registration(old._json(originals.read(spec.get("registration"))))
    plan = require_schema(spec.get("plan"), "capture-plan")
    if registration.get("capturePlan") != plan or registration.get("arms") != plan.get("arms"):
        raise ValueError("exact plan and arms must be frozen into source registration")
    code_root = Path(__file__).resolve().parents[2]
    descriptor, actors = freeze_protocol(code_root)
    if plan.get("actorSha256") != actors or any(name in plan for name in ("normalizerSha256", "assessorSha256")):
        raise ValueError("complete protocol2 actors must be frozen before capture")
    for name in ("exposureEvidence", "calibrationEvidence"):
        old._object(spec.get(name, {}), name)
    exposure = spec.get("exposureEvidence", {})
    expected = {event["evidenceSha256"] for event in registration.get("exposureLineage", {}).get("history", [])}
    if exposure.keys() != expected:
        raise ValueError("exposure evidence differs from source freeze")
    if registration["designation"] == "heldout" or "exposureLineage" in registration:
        failures = old.exposure_failures(registration, lambda digest: originals.read(exposure[digest]))
        if failures:
            raise ValueError("exposure lineage rejected: " + "; ".join(failures))
    old.verify_calibration_evidence(registration, spec.get("calibrationEvidence", {}), originals.read)
    panels = read_panels(registration, spec.get("panels"), originals.read, actors, external=True)
    if panels is not None:
        for source in panels["registrations"].values():
            old.verify_calibration_evidence(source, spec.get("calibrationEvidence", {}), originals.read)
    if (old._review_failures(registration.get("independentReview"), "source registration")
            or old.calibration_review_failures(registration)
            or ("developmentCalibration" not in registration
                and registration.get("sourceReview", {}).get("blindToOutputs") is not True)
            or registration["independentReview"].get("reviewer") == registration.get("sourceReview", {}).get("author")):
        raise ValueError("completed independent output-blind source review is required")
    started = datetime.now(timezone.utc).isoformat()
    if old._timestamp(registration["frozenAt"]) >= old._timestamp(started):
        raise ValueError("registration must be frozen before capture")
    roots = old._object(spec.get("sourceRoots"), "source roots")
    if roots.keys() != {repo["id"] for repo in registration["repositories"]}:
        raise ValueError("source roots differ from frozen repositories")
    for root in roots.values():
        if not Path(old._text(root, "absolute source root")).is_absolute():
            raise ValueError("source roots must be absolute")
    roots = {name: str(Path(path).resolve(strict=True)) for name, path in roots.items()}
    for item in (*registration["repositories"], *registration["tasks"]):
        capture.key(item["id"])
    old._verify_sources(registration, roots)
    target = Path(destination).resolve()
    protected = [code_root, *(Path(path) for path in roots.values())]
    if any(target.is_relative_to(path) or path.is_relative_to(target) for path in protected):
        raise ValueError("capture destination must be external to product/source checkouts")
    tools = [old._object(tool, "frozen tool")
        for tool in old._array(plan.get("tools"), "frozen tools", 2, 2)]
    if {old._choice(tool.get("id"), old.TOOLS, "tool") for tool in tools} != set(old.TOOLS):
        raise ValueError("complete unique native tool matrix required")
    arms = [old._object(arm, "frozen arm")
        for arm in old._array(plan.get("arms"), "frozen arms", 2, 2)]
    if {arm.get("id") for arm in arms} != set(old.ARMS):
        raise ValueError("complete unique arm matrix required")
    default = next(arm for arm in arms if arm["id"] == "default")
    normalized = next(arm for arm in arms if arm["id"] == "normalized")
    if default != dict(id="default") or (not normalized.keys() <= {
            "id", "budgetBytes", "measure", "overflow", "compassView", "configuredComparison"}
            or normalized.get("measure") != "utf8_bytes" or normalized.get("overflow") != "fail"):
        raise ValueError("unchanged default and frozen byte-budget arm required")
    old._integer(normalized.get("budgetBytes"), "byte budget", 128, 1024**2)
    view = old._choice(normalized.get("compassView", "default"), ("default", "ranked"), "Compass view")
    if "configuredComparison" in normalized:
        old._boolean(normalized["configuredComparison"], "configured comparison")
    if view == "ranked" and normalized.get("configuredComparison") is not True:
        raise ValueError("ranked arm requires configuredComparison=true")
    old._integer(plan.get("timeoutSeconds"), "query timeout", 1, 300)
    measurement = old._object(plan.get("measurement"), "frozen RSS measurement")
    style = old._choice(measurement.get("style"), ("darwin-l", "gnu-v"), "RSS style")
    system = platform.system()
    if system not in ("Darwin", "Linux") or style != ("darwin-l" if system == "Darwin" else "gnu-v"):
        raise ValueError("native capture requires matching Darwin/Linux RSS measurement")
    originals.read(measurement.get("executable"), old.MAX_GRAPH_BYTES)
    originals.read(plan.get("environmentExecutable"), old.MAX_GRAPH_BYTES)
    capture.child_environment(plan, target)
    dependencies, dependency_bytes = {}, 0
    for tool in tools:
        old.query_exit_contract(tool)
        originals.read(tool.get("executable"), old.MAX_GRAPH_BYTES)
        environment = old._object(old._json(originals.read(tool.get("environment"))), "tool environment")
        for dependency in old._array(environment.get("files"), "dependency files", MAX_DEP_FILES, 1):
            path = old._text(old._object(dependency, "dependency").get("path"), "dependency path")
            if path not in dependencies and len(dependencies) >= MAX_DEP_FILES:
                raise ValueError("dependency count bound exceeded before read")
            data = originals.read(dependency, min(old.MAX_GRAPH_BYTES,
                dependencies.get(path, MAX_DEP_BYTES - dependency_bytes)))
            if path not in dependencies:
                dependencies[path] = len(data)
                dependency_bytes += len(data)
        if tool.get("nativeOffline") is not True:
            raise ValueError("native offline tools must be explicitly attested")
        for name in ("version", "commit"):
            old._text(tool.get(name), "tool " + name, 128)
        settings = old._object(tool.get("settings"), "tool settings")
        for name in ("scopePolicy", "inferenceProfile", "clustering", "queryInterface"):
            old._text(settings.get(name), "tool setting " + name)
        if tool["id"] == "graphify" or "releaseVerification" in tool:
            release = old._object(old._json(originals.read(tool.get("releaseVerification"))), "release verification")
            if ((release.get("version"), release.get("commit")) != (tool["version"], tool["commit"])
                    or old._timestamp(release.get("checkedAt")) > old._timestamp(started)
                    or release.get("source") != "https://github.com/Graphify-Labs/graphify/releases"):
                raise ValueError("release verification differs from source-frozen tool identity")
    graphs = [old._object(graph, "frozen graph")
        for graph in old._array(plan.get("graphs"), "frozen graphs", 64, 2)]
    expected = {(repo["id"], tool) for repo in registration["repositories"] for tool in old.TOOLS}
    if len(graphs) != len(expected) or {(g.get("repository"), g.get("tool")) for g in graphs} != expected:
        raise ValueError("complete unique graph matrix required")
    for entry in graphs:
        graph = old._json(originals.read(entry.get("artifact"), old.MAX_GRAPH_BYTES))
        old.GraphIndex(graph, entry["tool"])
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
        build = old._json(originals.read(entry.get("buildCapture")))
        if not old.verify_build_capture(build, originals.read):
            raise ValueError("successful bounded raw graph build capture required")
        repo = next(repo for repo in registration["repositories"] if repo["id"] == entry["repository"])
        inventory = old._object(old._json(originals.read(entry.get("sourceInventory"))), "source inventory")
        if (inventory.get("files") != repo["files"] or entry.get("sourceCommit") != repo["commit"]
                or entry.get("sourceCleanBeforeAndAfter") is not True):
            raise ValueError("graph inventory/pin/cleanliness differs from source freeze")
    # Validate every eventual native argv, including prefixes/flags, before the
    # first query. Copied graph paths are selected later without changing flags.
    for task in registration["tasks"]:
        old._text(task["question"], "frozen question", 16384)
        for arm in old.ARMS:
            for tool in tools:
                argv = capture.launch(plan, capture.command(plan, tool, arm, task["question"], target / "graph.json"), target)
                old._array(list(argv), "native argv", 128, 1)
                for argument in argv:
                    old._text(argument, "native argument", 16384)
    return originals, registration, roots, started, descriptor, actors


def collect_v2(spec, destination, *, runner=process.run_bounded, _input_reference=None):
    """Capture once, retain the entire matrix, assess pending review, then seal.

    This internal API is not authorization for a real comparison. The public
    version1 collector still rejects version2. COMPLETE means coherent capture only;
    pending suggestions cannot pass independent output review or superiority.
    """
    spec = old._json(capture.encoded(spec))  # snapshot caller-owned mutable data
    originals, registration, roots, started, descriptor, actors = _preflight(spec, destination, _input_reference)
    plan, code_root = spec["plan"], Path(__file__).resolve().parents[2]
    bundle = CaptureBundle(destination, originals)
    registration_ref = bundle.copy("registration.json", spec["registration"])
    input_ref = bundle.put("capture-input.json", spec)
    invocation = (dict(original=_input_reference,
        artifact=bundle.copy("capture-input-invocation.json", _input_reference))
        if _input_reference is not None else None)
    protocol = retain_protocol(bundle, code_root, descriptor, actors)
    panels = retain_panels(bundle, spec.get("panels"))
    modules = {item["module"]: item["artifact"] for item in protocol["files"]}
    histories = {name: {digest: bundle.copy(f"provenance/{name}/{digest}", reference)
        for digest, reference in sorted(spec.get(name, {}).items())}
        for name in ("exposureEvidence", "calibrationEvidence")}
    tools = []
    for tool in plan["tools"]:
        name = tool["id"]
        record = {field: tool[field] for field in ("id", "version", "commit", "nativeOffline", "settings")}
        for field in ("executable", "environment", "releaseVerification"):
            if field in tool:
                record[field] = bundle.copy(f"provenance/{name}-{field}", tool[field],
                    old.MAX_GRAPH_BYTES if field == "executable" else old.MAX_JSON_BYTES)
        if "queryExitContract" in tool:
            record["queryExitContract"] = tool["queryExitContract"]
        record["normalizer"] = modules[ROLES["normalizer"]]
        files = old._json(originals.read(tool["environment"]))["files"]
        record["dependencies"] = [dict(original=reference, artifact=bundle.copy(
            f"provenance/{name}-dependencies/{number}", reference, old.MAX_GRAPH_BYTES))
            for number, reference in enumerate(files)]
        tools.append(record)
    graphs, graph_paths = [], {}
    for entry in plan["graphs"]:
        prefix = f"graphs/{entry['repository']}-{entry['tool']}"
        record = {field: entry[field] for field in ("repository", "tool", "sourceCommit", "sourceCleanBeforeAndAfter")}
        for field in ("artifact", "sourceInventory"):
            relative = prefix + ("/graph.json" if field == "artifact" else "/inventory.json")
            record[field] = bundle.copy(relative, entry[field], old.MAX_GRAPH_BYTES if field == "artifact" else old.MAX_JSON_BYTES)
        if "sourceQueryArtifact" in entry:
            record["sourceQueryArtifact"] = bundle.copy(prefix + "/" + old.SOURCE_QUERY_FILE,
                entry["sourceQueryArtifact"], old.MAX_SOURCE_QUERY_BYTES)
        record["originalBuildCapture"] = bundle.copy(prefix + "/original-build.json", entry["buildCapture"])
        build = old._json(originals.read(entry["buildCapture"]))
        for stream in ("stdout", "stderr"):
            build[stream] = bundle.copy(prefix + "/build." + stream, build[stream])
        record["buildCapture"] = bundle.put(prefix + "/build.json", build)
        graphs.append(record)
        graph_paths[entry["repository"], entry["tool"]] = bundle.root / record["artifact"]["path"]
    bundle.graphs = graphs
    environment = capture.child_environment(plan, bundle.root)
    for name in capture.ENV_PATH_KEYS:
        Path(environment[name]).mkdir(parents=True, exist_ok=True)

    def source_state(phase):
        for repo in registration["repositories"]:
            root = Path(roots[repo["id"]])
            options = ("--no-optional-locks", "-c", "core.fsmonitor=false", "-c", "core.untrackedCache=false")
            for name, args in (("head", ("rev-parse", "HEAD")), ("status", ("status", "--porcelain", "--untracked-files=all"))):
                _, record, raw = bundle.execute(("git", *options, "-C", str(root), *args),
                    f"source-checks/{repo['id']}-{phase}-{name}.json", root, 15, runner)
                if record["exit_code"] or record["timed_out"] or record["output_limited"]:
                    raise ValueError("bounded source metadata verification failed")
                if raw["stdout"].decode("utf-8", errors="strict").strip() != (repo["commit"] if name == "head" else ""):
                    raise ValueError("source pin or cleanliness changed")
        old._verify_sources(registration, roots)

    source_state("before")
    # These guards run after copying and before any query. No mutable original
    # input or retained copy may silently substitute for its preflight bytes.
    bundle.recheck()
    verify_original_protocol(code_root, descriptor, actors)
    observations, pending, selections = [], [], []
    work = BundleWork()
    for task in sorted(registration["tasks"], key=lambda item: item["id"]):
        for arm in old.ARMS:
            for name in old.TOOLS:
                tool = next(tool for tool in plan["tools"] if tool["id"] == name)
                graph_path = graph_paths[task["repository"], name]
                graph = old.GraphIndex(old._json(old.read_bounded(graph_path, old.MAX_GRAPH_BYTES)), name)
                argv = capture.launch(plan, capture.command(plan, tool, arm, task["question"], graph_path), bundle.root)
                prefix = f"queries/{task['id']}/{name}-{arm}"
                reference, record, raw = bundle.execute(argv, prefix + ".json", bundle.root, plan["timeoutSeconds"], runner)
                outcome = old.query_outcome(tool, record["exit_code"], raw["stdout"],
                    timed_out=record["timed_out"], limited=record["output_limited"])
                normalized = normalize_v2(raw["stdout"], name, graph, task=task["id"], arm=arm,
                    source_registration_sha256=registration_ref["sha256"], actors=actors, bundle=work,
                    failed=outcome == "failure", no_match_outcome=outcome == "no_match",
                    limited=record["timed_out"] or record["output_limited"])
                initial_ref = bundle.put(prefix + "-normalized.json", normalized)
                identity = dict(task=task["id"], tool=name, arm=arm)
                observations.append(dict(identity, captures=[reference], normalized=initial_ref,
                    condition="fresh_cli", peakRssBytes=parse_rss_v2(raw["stderr"], plan["measurement"]["style"])))
                label = (_pending_judgment(normalized) if normalized["normalizationStatus"] != "rejected"
                    else dict(schema=SCHEMAS["judgments"], semanticCoverageComplete=False, emptyConfirmed=False,
                        lineCoverage=[], **pack_claim_batches([])))
                label.update(identity, reviewedNormalizedSha256=initial_ref["sha256"], candidates=[],
                    independentReview=dict(capture.PENDING))
                # Keep global state reference-only. Large per-row labels are
                # spooled before the run hash exists, then bound after sealing.
                draft = bundle.put(prefix + "-pending-partition.json", label)
                pending.append(dict(identity, draft=draft, path=prefix + "-judgments.json"))
                selections.append(dict(identity, initialNormalizedRef=initial_ref, reviewedNormalizedRef=initial_ref,
                    rawStdoutSha256=old.sha(raw["stdout"]), sourceRegistrationSha256=registration_ref["sha256"], actorSha256=actors))
                del graph, raw, normalized, label
    source_state("after")
    bundle.recheck()
    verify_original_protocol(code_root, descriptor, actors)
    run = bundle.put("run.json", dict(schema=SCHEMAS["capture"], registrationSha256=registration_ref["sha256"],
        startedAt=started, completedAt=datetime.now(timezone.utc).isoformat(), protocolBinding=protocol,
        captureInput=input_ref, captureInputInvocation=invocation, panels=panels,
        assessor=modules[ROLES["assessor"]], tools=tools, graphs=graphs, arms=plan["arms"],
        observations=observations, childEnvironment=environment,
        measurementExecutable=bundle.copy("provenance/measurement-executable", plan["measurement"]["executable"], old.MAX_GRAPH_BYTES),
        environmentExecutable=bundle.copy("provenance/environment-executable", plan["environmentExecutable"], old.MAX_GRAPH_BYTES),
        collectionOrder="task_id_then_default_normalized_then_tool_id; no retries or followups",
        measurementScope="fresh isolated CLI processes, unflushed OS cache; time wrapper RSS; no cold-cache claim"))
    reviews = []
    for row in pending:
        label = old._json(old.read_bounded(bundle.root / row["draft"]["path"], old.MAX_JSON_BYTES))
        if old.sha(capture.encoded(label)) != row["draft"]["sha256"]:
            raise ValueError("pending partition changed before run binding")
        label["runSha256"] = run["sha256"]
        reviews.append(dict(task=row["task"], tool=row["tool"], arm=row["arm"], judgmentRef=bundle.put(row["path"], label)))
    for selection in selections:
        selection["originalRunSha256"] = run["sha256"]
    review_ref = bundle.put("reviews-pending.json", dict(schema=SCHEMAS["judgments"], runSha256=run["sha256"], observations=reviews))
    manifest = dict(schema=SCHEMAS["review-input"], registration=registration_ref, run=run, reviews=review_ref,
        normalizedSelections=selections, sourceRoots=roots, graphQualification=[], **histories)
    bundle.put("review-input.json", manifest)
    def publish_report(path, data):
        return bundle.put(path.relative_to(bundle.root).as_posix(), data)
    stored = score_file_v2(bundle.root / "review-input.json", bundle.root / "report-provisional.json",
        _publish=publish_report)
    report = stored["assessment"]
    if report.get("eligibleForSuperiorityClaim") is not False:
        raise ValueError("pending output review cannot establish qualification")
    source_state("publication")
    bundle.recheck()
    verify_original_protocol(code_root, descriptor, actors)
    bundle.put("COMPLETE.json", dict(status="captured_pending_independent_output_review", qualificationEstablished=False,
        note="Capture publication only; pending output review cannot establish comparative quality."))
    return dict(bundle=str(bundle.root), observations=len(observations),
        report=str(bundle.root / "report-provisional.json"), qualificationEstablished=False)


def collect_file_v2(path, destination, *, runner=process.run_bounded):
    """Bind the actual invocation file, preserving its original bytes/path.

    The same file is tracked by the original-input reader before/after queries,
    assessment and final publication. The assessor also verifies its retained
    copy. The explicit version2 developer entry invokes this file-backed path.
    """
    path = Path(path).absolute()
    data = old.read_bounded(path, old.MAX_JSON_BYTES)
    return collect_v2(old._json(data), destination, runner=runner,
        _input_reference=dict(path=str(path), sha256=old.sha(data)))
