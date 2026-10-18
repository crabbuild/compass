"""Bounded, developer-only Plan 029 capture; never installs or builds a tool.

A pre-output registration freezes the exact capturePlan. Queries use prebuilt
native graphs and explicit executables. Parsed spans are review suggestions,
not source facts: unresolved text and missing owners remain in a review queue.
Run ``python3 -m benchmarks.agent_query.broad_query_capture --help``.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import platform
import re
import sys
from typing import Callable

from benchmarks.agent_query.broad_query_oracle import publish_exclusive
from benchmarks.agent_query.broad_query_review import (
    ARMS, TOOLS, GraphIndex, Identity, MANIFEST_SCHEMA, MAX_GRAPH_BYTES,
    MAX_JSON_BYTES, NORMALIZED_SCHEMA, REVIEW_SCHEMA, RUN_SCHEMA,
    _array, _boolean, _choice, _digest, _integer, _json, _object, _relative,
    _review_failures, _text, _timestamp, _verify_sources, main as review_main,
    sha, validate_registration, verify_build_capture, exposure_failures,
)
from benchmarks.agent_query.community_tasks import read_bounded
from benchmarks.agent_query.runner import _terminal_symbol, run_bounded

SCHEMA = "compass.broad-query-capture-plan/1"
INPUT_SCHEMA = "compass.broad-query-capture-input/1"
PENDING = dict(status="pending", reviewer=None, reviewerType=None,
               sourceAware=False, independentOfAuthor=False, toolLabelsBlinded=False)
SAFE_KEY = re.compile(r"[a-zA-Z0-9][a-zA-Z0-9_.-]{0,127}\Z")
GRAPHIFY_NODE = re.compile(r"^NODE (?P<label>.*?) \[src=(?P<file>.*?) loc=L(?P<line>[0-9]+)(?: |\])")
COMPASS_NODE = re.compile(r"^(?:NODE|ALSO MATCHED) (?P<label>.*?) (?:\[[^\]]+\] |@ )(?P<file>.+):(?P<line>[0-9]+):[0-9]+-[0-9]+:[0-9]+$")
COMPASS_SEED = re.compile(r"^SEED (?P<label>.*?) \[(?:exact|approximate|auto-picked)\] @ (?P<file>.+):(?P<line>[0-9]+):[0-9]+-[0-9]+:[0-9]+$")
COMPASS_EVIDENCE_NODE = re.compile(r"^Node: (?P<id>\S+) \[[^\]]+\] (?P<label>.*?) @ (?P<file>.+):(?P<line>[0-9]+):[0-9]+-[0-9]+:[0-9]+(?: |$)")
RANKED_NODE = re.compile(r"^(?P<rank>[1-9][0-9]*)\. (?P<label>.*?) @ (?P<file>.+):(?P<line>[0-9]+) \[relevance=")
AGENT_NODE = re.compile(r"^- (?P<label>.*?) \[[^\]]+\] (?P<file>.+):L(?P<line>[0-9]+):[0-9]+-L[0-9]+:[0-9]+$")
COMPASS_EDGE = re.compile(r"^EDGE (?P<source>.*?) --(?P<kind>[a-z_]+)--> (?P<target>.*?) \[[^\]]+\]$")
GRAPHIFY_EDGE = re.compile(r"^EDGE (?P<source>.*?) --(?P<kind>[a-z_]+) \[[^\]]+\]--> (?P<target>.*?)$")
PRINTED_EDGE_SITE = re.compile(r"(?P<file>.+):L(?P<line>[1-9][0-9]*)(?:-L(?P<end>[1-9][0-9]*))?\Z")


def encoded(value: dict | list) -> bytes:
    data = (json.dumps(value, indent=2, sort_keys=True, allow_nan=False) + "\n").encode()
    if len(data) > MAX_JSON_BYTES:
        raise ValueError("capture document exceeds bounded JSON size")
    return data


def key(value: str) -> str:
    if not isinstance(value, str) or not SAFE_KEY.fullmatch(value):
        raise ValueError("unsafe artifact key")
    return value


def external(ref: dict, maximum: int = MAX_JSON_BYTES) -> tuple[Path, bytes]:
    ref = _object(ref, "external input reference")
    path = Path(_text(ref.get("path"), "absolute external input path"))
    if not path.is_absolute():
        raise ValueError("external inputs require absolute paths")
    # Preserve executable invocation paths (notably Python venv symlinks) and
    # recheck that same path after capture. Resolving here would retain only
    # the old target and miss a later symlink retarget.
    data = read_bounded(path, maximum)
    if sha(data) != _digest(ref.get("sha256")):
        raise ValueError("external input digest drift")
    return path, data


class Bundle:
    """No-clobber publication; incomplete directories never contain COMPLETE."""
    def __init__(self, root: Path):
        self.root = root.resolve()
        self.root.mkdir(parents=True, exist_ok=False)
        self.total = 0
        self.paths = 0
        self.inputs: dict[Path, tuple[str, int]] = {}

    def put(self, relative: str, value: dict | list | bytes) -> dict:
        path = self.root / _relative(relative)
        data = value if isinstance(value, bytes) else encoded(value)
        self.total += len(data)
        self.paths += 1
        if self.paths > 65536:
            raise ValueError("capture bundle exceeds artifact count bound")
        if self.total > 2 * 1024**3:
            raise ValueError("capture bundle exceeds two GiB bound")
        path.parent.mkdir(parents=True, exist_ok=True)
        publish_exclusive(path, data)
        return dict(path=relative, sha256=sha(data))

    def copy(self, relative: str, ref: dict, maximum: int = MAX_JSON_BYTES) -> dict:
        path, data = external(ref, maximum)
        self.inputs[path] = (sha(data), maximum)
        return self.put(relative, data)

    def recheck(self):
        for path, (digest, maximum) in self.inputs.items():
            if sha(read_bounded(path, maximum)) != digest:
                raise ValueError("capture input changed while collecting")


def _span(start: int, data: bytes) -> dict:
    return dict(startByte=start, endByte=start + len(data), sha256=sha(data))


def _printed_identity(label: str, file: str, line: int) -> Identity | None:
    """Use only printed owner text. Missing receiver owners need source review."""
    name = _terminal_symbol(label)
    if not name:
        return None
    clean = re.sub(r"\(.*\)$", "", label.strip())
    if clean.startswith("."):
        return None  # Graphify method syntax does not print its lexical receiver.
    if "::" in clean:
        owner = clean.rsplit("::", 1)[0]
    elif "." in clean:
        owner = clean.rsplit(".", 1)[0]
    else:
        return None  # unqualified may be module-level OR an unprinted member
    return Identity.parse(dict(file=file, startLine=line, symbol=name, owner=owner))


def _printed_edge_site(line: str) -> tuple[str, dict | None, str | None]:
    """Separate an actual printed citation, never a source witness or graph site."""
    if " at=" not in line:
        return line, None, None
    body, citation = line.rsplit(" at=", 1)
    match = PRINTED_EDGE_SITE.fullmatch(citation)
    if match:
        try:
            file = _relative(match["file"])
            start = int(match["line"])
            end = int(match["end"] or match["line"])
            if end >= start:
                return body, dict(file=file, startLine=start, endLine=end), None
        except ValueError:
            pass
    return body, None, citation


def normalize(output: bytes, tool: str, graph: GraphIndex, *, task: str, arm: str,
              failed: bool = False, limited: bool = False) -> dict:
    """Extract printed anchors/edges without reading oracle facts or relevance.

    This function intentionally has no source-registration argument. Graph-only
    neighbors do not enter ranked/claims. A reviewer must resolve missing owners
    and judge every behavioral assertion against source and actual output spans.
    """
    output.decode("utf-8", errors="strict")
    lines = output.splitlines(keepends=True)
    if len(lines) > 65536 or any(len(line) > 64 * 1024 for line in lines):
        raise ValueError("printed line/count exceeds parser bounds; raw output retained")
    ranked, claims, queue, labels, seen = [], [], [], {}, set()
    offset = 0
    ranked_format = False
    printed_ranks = []
    resolved_ranks = []
    for line_bytes in lines:
        line = line_bytes.decode().rstrip("\r\n")
        span = _span(offset, line_bytes.rstrip(b"\r\n")) if line.strip() else None
        offset += len(line_bytes)
        match = (GRAPHIFY_NODE.match(line) if tool == "graphify" else
                 COMPASS_EVIDENCE_NODE.match(line) or COMPASS_NODE.match(line) or COMPASS_SEED.match(line)
                 or RANKED_NODE.match(line) or AGENT_NODE.match(line))
        if not match:
            continue
        if "rank" in match.groupdict():
            ranked_format = True
            printed_ranks.append(int(match["rank"]))
        file, lineno, label = match["file"], int(match["line"]), match["label"]
        try:
            file = _relative(file)
        except ValueError:
            queue.append(dict(kind="invalid_printed_anchor", text=line, span=span))
            continue
        terminal = _terminal_symbol(label)
        matches = graph.anchors.get((file, lineno, terminal), []) if terminal else []
        printed_id = match.groupdict().get("id")
        if printed_id and matches != [printed_id]:
            matches = []
        identity = _printed_identity(label, file, lineno)
        item = dict(kind="printed_declaration", printedLabel=label, file=file, startLine=lineno,
                    graphMatches=matches, span=span, ownerPrinted=identity is not None,
                    note="Graph matching diagnoses identity; it does not supply an unprinted owner or source fact.")
        queue.append(item)
        labels.setdefault(label, []).append(item)
        if identity is None or len(matches) != 1:
            continue
        item["resolvedIdentity"] = identity.record()
        if identity in seen:
            continue
        seen.add(identity)
        if len(ranked) >= 512:
            raise ValueError("printed candidates exceed normalization bound")
        candidate = dict(identity=identity.record(), nodeId=matches[0], firstPage=True, span=span)
        ranked.append(candidate)
        if "rank" in match.groupdict():
            resolved_ranks.append(int(match["rank"]))
        # The assertion is merely the printed declaration, never its behavior.
        claims.append(dict(id=f"declaration-{len(claims)+1}", identity=identity.record(), span=span,
                           assertionKind="declaration_identity_only"))
    offset = 0
    connectivity_claims = {}
    for line_bytes in lines:
        line = line_bytes.decode().rstrip("\r\n")
        span = _span(offset, line_bytes.rstrip(b"\r\n")) if line.strip() else None
        offset += len(line_bytes)
        edge_line, printed_site, malformed_site = _printed_edge_site(line)
        edge = (GRAPHIFY_EDGE if tool == "graphify" else COMPASS_EDGE).match(edge_line)
        if not edge:
            # All unmatched text remains visible to reviewers, including prose
            # assertions and headers. It cannot silently disappear from an
            # alleged complete normalization or a precision denominator.
            if line.strip() and not any(item.get("span") == span for item in queue):
                if len(queue) >= 65536:
                    raise ValueError("output review queue exceeds line bound")
                queue.append(dict(kind="unparsed_printed_text", span=span, text=line))
            continue
        ends = [labels.get(edge[end], []) for end in ("source", "target")]
        # Repeating the same printed declaration (seed plus node inventory)
        # supplies no new claim and does not make that endpoint ambiguous.
        # Every occurrence must resolve to that same fully printed identity;
        # an unresolved or conflicting occurrence still prevents resolution.
        valid = [items[0].get("resolvedIdentity") if items and all(
            item.get("resolvedIdentity") == items[0].get("resolvedIdentity") for item in items
        ) else None for items in ends]
        item = dict(kind="printed_relation", sourceLabel=edge["source"], targetLabel=edge["target"],
                    relation=edge["kind"], span=span, text=line, printedSite=printed_site,
                    malformedPrintedSite=malformed_site,
                    note="Printed citation only; independent source review must verify its witness. Initial connectivity claims have no occurrence/source text.")
        queue.append(item)
        if all(valid) and malformed_site is None:
            relation = dict(source=valid[0], target=valid[1], kind=edge["kind"],
                            direction="source_to_target", occurrences=[])
            semantic = (Identity.parse(valid[0]), Identity.parse(valid[1]), edge["kind"])
            identifier = connectivity_claims.get(semantic)
            item["coalescedConnectivityClaim"] = identifier is not None
            if identifier is None:
                identifier = f"relation-{len(claims)+1}"
                connectivity_claims[semantic] = identifier
                claims.append(dict(id=identifier, identity=valid[0], relation=relation, span=span))
            item["connectivityClaimId"] = identifier
    if len(claims) > 128:
        raise ValueError("printed claims exceed normalization bound")
    # Empty prose is not evidence of an unsupported-subject answer. Explicit
    # native no-result signals are suggestions pending independent disposition review.
    no_match = any(signal in output.decode() for signal in (
        "No matching nodes found", "No node matching", "No supporting indexed evidence found.",
        "No matching indexed", "no_match"))
    declarations = any(item["kind"] == "printed_declaration" for item in queue)
    disposition = "limit" if limited else "error" if failed else "answered" if declarations else "abstain" if no_match else "error"
    # Do not compress rank gaps when one printed candidate lacks an owner or
    # has ambiguous graph identity. An independent reviewer must resolve it.
    exact_ranking = (ranked_format and printed_ranks == list(range(1, len(printed_ranks) + 1))
                     and resolved_ranks == printed_ranks and len(ranked) == len(printed_ranks))
    return dict(schema=NORMALIZED_SCHEMA, task=task, tool=tool, arm=arm,
                disposition=disposition, rankingKind="retrieval" if exact_ranking else "display",
                admission=None, ranked=ranked, claims=claims,
                normalizationComplete=False, reviewQueue=queue,
                normalizationCaveats=["Pending independent source-aware output review.",
                    "Unqualified or missing owners remain unresolved; no oracle identities are supplied.",
                    "Display order is not retrieval rank; unexposed admission is not instrumented.",
                    "Behavioral facts require an actual assertion span; declaration identity alone is insufficient."])


def command(plan: dict, tool: dict, arm: str, question: str, graph: Path) -> tuple[str, ...]:
    prefix = tuple(_text(arg, "command prefix", 4096) for arg in
                   _array(tool.get("argvPrefix", []), "explicit executable arguments", 16))
    result = [tool["executable"]["path"], *prefix, "query", question, "--graph", str(graph)]
    arm_plan = next(a for a in plan["arms"] if a["id"] == arm)
    view = arm_plan.get("compassView", "default")
    if tool["id"] == "compass" and view == "ranked":
        result.extend(["--view", "ranked"])
    elif view != "default" and tool["id"] == "compass":
        raise ValueError("unsupported predeclared Compass view")
    if arm == "normalized":
        # Equal native token-budget inputs do not imply equal native estimators:
        # Graphify uses three Unicode characters per token. Actual UTF-8 bytes
        # determine the separately frozen strict cost gate for both tools.
        budget = _integer(arm_plan.get("budgetBytes"), "normalized bytes", 128, 1024**2)
        result.extend(["--text-budget" if tool["id"] == "compass" else "--budget", str(budget // 4)])
    return tuple(result)


ENV_PATH_KEYS = {"HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME", "XDG_STATE_HOME", "APPDATA", "LOCALAPPDATA", "TMPDIR"}
ENV_KEYS = ENV_PATH_KEYS | {"PATH", "GRAPHIFY_NO_AUTO_REFRESH", "GRAPHIFY_QUERY_LOG_DISABLE",
                           "PYTHONDONTWRITEBYTECODE", "PYTHONNOUSERSITE", "PYTHONHASHSEED", "LC_ALL", "LANG", "TZ"}


def child_environment(plan: dict, root: Path) -> dict[str, str]:
    declared = _object(plan.get("childEnvironment"), "frozen isolated child environment")
    if declared.get("inherit") is not False or set(declared) - ENV_KEYS - {"inherit"}:
        raise ValueError("child environment must be an explicit nonsecret allowlist with inherit=false")
    if not ENV_PATH_KEYS <= set(declared):
        raise ValueError("all child home/cache/temp locations must be isolated")
    result = {}
    for name, value in declared.items():
        if name == "inherit":
            continue
        value = _text(value, "nonsecret environment setting", 4096).replace("{artifactRoot}", str(root.resolve()))
        if name in ENV_PATH_KEYS:
            path = Path(value).resolve()
            if not Path(value).is_absolute() or not path.is_relative_to(root.resolve()):
                raise ValueError("child home/cache/temp path escapes new external artifact root")
        result[name] = value
    if result.get("PATH") != "/usr/bin:/bin":
        raise ValueError("native capture requires the frozen minimal system PATH")
    if any(result.get(name) != "1" for name in ("GRAPHIFY_NO_AUTO_REFRESH", "GRAPHIFY_QUERY_LOG_DISABLE")):
        raise ValueError("query capture must disable comparator graph mutation/logging explicitly")
    return result


def launch(plan: dict, argv: tuple[str, ...], root: Path) -> tuple[str, ...]:
    """Freeze the exact sanitized environment and common RSS wrapper in argv."""
    env = child_environment(plan, root)
    measurement = plan["measurement"]
    return (str(Path(plan["environmentExecutable"]["path"]).resolve()), "-i",
            *(name + "=" + value for name, value in sorted(env.items())),
            str(Path(measurement["executable"]["path"]).resolve()),
            "-l" if measurement["style"] == "darwin-l" else "-v", *argv)


def rss(stderr: bytes, style: str) -> int | None:
    text = stderr.decode("utf-8", errors="replace")
    pattern = (r"^\s*([0-9]+)\s+maximum resident set size\s*$" if style == "darwin-l" else
               r"^\s*Maximum resident set size \(kbytes\):\s*([0-9]+)\s*$")
    matches = re.findall(pattern, text, re.M)
    return int(matches[0]) * (1 if style == "darwin-l" else 1024) if len(matches) == 1 else None


def collect(spec: dict, destination: Path, *, runner: Callable = run_bounded) -> dict:
    """Execute a complete frozen matrix, retaining failed and over-budget rows."""
    if spec.get("schema") != INPUT_SCHEMA:
        raise ValueError("unsupported capture-input schema")
    _, registration_data = external(spec.get("registration"))
    registration = validate_registration(_json(registration_data))
    plan = _object(spec.get("plan"), "capture plan")
    if plan.get("schema") != SCHEMA or registration.get("capturePlan") != plan:
        raise ValueError("exact capture plan was not frozen into source registration")
    if registration.get("arms") != plan.get("arms"):
        raise ValueError("arm settings differ from frozen source registration")
    if plan.get("normalizerSha256") != sha(read_bounded(Path(__file__), MAX_JSON_BYTES)):
        raise ValueError("normalizer code was not frozen before capture")
    from benchmarks.agent_query import broad_query_review
    if plan.get("assessorSha256") != sha(read_bounded(Path(broad_query_review.__file__), MAX_JSON_BYTES)):
        raise ValueError("assessor code was not frozen before capture")
    exposure_refs = _object(spec.get("exposureEvidence", {}), "exposure evidence references")
    expected_exposure = {event["evidenceSha256"] for event in
                         registration.get("exposureLineage", {}).get("history", [])}
    if set(exposure_refs) != expected_exposure:
        raise ValueError("exposure evidence references differ from frozen history")
    if registration["designation"] == "heldout" or "exposureLineage" in registration:
        def read_exposure(digest):
            _, data = external(exposure_refs[digest])
            return data
        failures = exposure_failures(registration, read_exposure)
        if failures:
            raise ValueError("exposure lineage rejected: " + "; ".join(failures))
    # Source acceptance is required even for an underpowered diagnostic capture.
    # Statistical/graph eligibility is assessed separately after measurements.
    if _review_failures(registration.get("independentReview"), "source registration"):
        raise ValueError("source registration lacks completed independent source review")
    if registration.get("sourceReview", {}).get("blindToOutputs") is not True:
        raise ValueError("source-first output-blinded authoring is required")
    if registration.get("independentReview", {}).get("reviewer") == registration.get("sourceReview", {}).get("author"):
        raise ValueError("source author cannot independently review their own labels")
    started = datetime.now(timezone.utc).isoformat()
    if _timestamp(registration["frozenAt"]) >= _timestamp(started):
        raise ValueError("registration must be frozen before collection")
    roots = {name: str(Path(path).resolve(strict=True)) for name, path in
             _object(spec.get("sourceRoots"), "source roots").items()}
    _verify_sources(registration, roots)
    repo_root = Path(__file__).resolve().parents[2]
    target = destination.resolve()
    protected = [repo_root, *(Path(path) for path in roots.values())]
    if any(target.is_relative_to(path) or path.is_relative_to(target) for path in protected):
        raise ValueError("capture destination must be external to product and source checkouts")
    tools = _array(plan.get("tools"), "frozen tools", 2, 2)
    if {t.get("id") for t in tools} != set(TOOLS):
        raise ValueError("both explicit native tools are required")
    arms = _array(plan.get("arms"), "frozen arms", 2, 2)
    if {a.get("id") for a in arms} != set(ARMS):
        raise ValueError("default and normalized predeclared arms required")
    default = next(a for a in arms if a["id"] == "default")
    if default.get("compassView", "default") != "default":
        raise ValueError("default arm must use default native Compass query")
    normalized = next(a for a in arms if a["id"] == "normalized")
    if normalized.get("measure") != "utf8_bytes" or normalized.get("overflow") != "fail":
        raise ValueError("normalized arm requires byte cost and overflow=fail")
    _integer(normalized.get("budgetBytes"), "normalized byte budget", 128, 1024**2)
    if normalized.get("compassView", "default") == "ranked" and normalized.get("configuredComparison") is not True:
        raise ValueError("opt-in ranked primary arm must be explicitly labeled configuredComparison")
    timeout = _integer(plan.get("timeoutSeconds"), "query timeout", 1, 300)
    instrument = _object(plan.get("measurement"), "predeclared RSS instrumentation")
    style = _choice(instrument.get("style"), ("darwin-l", "gnu-v"), "RSS instrumentation")
    if platform.system() not in ("Darwin", "Linux"):
        raise ValueError("this developer runner requires the existing POSIX bounded subprocess backend")
    if style != ("darwin-l" if platform.system() == "Darwin" else "gnu-v"):
        raise ValueError("RSS instrumentation does not match this platform")
    external(instrument.get("executable"), MAX_GRAPH_BYTES)
    external(plan.get("environmentExecutable"), MAX_GRAPH_BYTES)
    isolated_environment = child_environment(plan, target)
    # Validate every immutable input before the first actual tool invocation.
    dependency_inputs = {}
    dependency_bytes = 0
    for tool in tools:
        external(tool.get("executable"), MAX_GRAPH_BYTES)
        _, environment = external(tool.get("environment"))
        dependencies = _array(_json(environment).get("files"), "environment/dependency files", 65536, 1)
        for dependency in dependencies:
            dependency_path, dependency_data = external(dependency, MAX_GRAPH_BYTES)
            if dependency_path not in dependency_inputs:
                dependency_inputs[dependency_path] = len(dependency_data)
                dependency_bytes += len(dependency_data)
            if len(dependency_inputs) > 65536 or dependency_bytes > 512 * 1024**2:
                raise ValueError("dependency inventory exceeds bounded count/byte totals")
        if tool.get("nativeOffline") is not True:
            raise ValueError("native offline tools must be explicitly attested")
        for name in ("scopePolicy", "inferenceProfile", "clustering", "queryInterface"):
            _text(tool.get("settings", {}).get(name), "predeclared " + name)
        if tool["id"] == "graphify":
            external(tool.get("releaseVerification"))
    graph_records = _array(plan.get("graphs"), "prebuilt graph inputs", 64, 2)
    expected = {(r["id"], tool) for r in registration["repositories"] for tool in TOOLS}
    if {(g.get("repository"), g.get("tool")) for g in graph_records} != expected or len(graph_records) != len(expected):
        raise ValueError("prebuilt graph matrix incomplete or duplicated")
    graph_indexes = {}
    for entry in graph_records:
        _, data = external(entry.get("artifact"), MAX_GRAPH_BYTES)
        graph_indexes[entry["repository"], entry["tool"]] = GraphIndex(_json(data), entry["tool"])
        _, build_data = external(entry.get("buildCapture"))
        if not verify_build_capture(_json(build_data), lambda ref: external(ref)[1]):
            raise ValueError("prebuilt graph lacks a successful bounded raw build capture")
        if entry.get("sourceCleanBeforeAndAfter") is not True:
            raise ValueError("prebuilt graph lacks before/after source-clean build attestation")
        _, inv = external(entry.get("sourceInventory"))
        repo = next(r for r in registration["repositories"] if r["id"] == entry["repository"])
        if _json(inv).get("files") != repo["files"] or entry.get("sourceCommit") != repo["commit"]:
            raise ValueError("prebuilt graph inventory/pin differs from registration")
    bundle = Bundle(target)
    registration_ref = bundle.copy("registration.json", spec["registration"])
    copied_exposure = {digest: bundle.copy("provenance/exposure/" + digest, exposure_refs[digest])
                       for digest in sorted(exposure_refs)}
    bundle.put("capture-input.json", spec)
    normalizer_ref = bundle.put("provenance/broad_query_capture.py", read_bounded(Path(__file__), MAX_JSON_BYTES))
    measurement_ref = bundle.copy("provenance/measurement-executable", instrument["executable"], MAX_GRAPH_BYTES)
    environment_launcher_ref = bundle.copy("provenance/environment-executable", plan["environmentExecutable"], MAX_GRAPH_BYTES)
    for name in ENV_PATH_KEYS:
        Path(isolated_environment[name]).mkdir(parents=True, exist_ok=True)
    # Save every dependency manifest and bind/recheck all recorded dependency
    # bytes. No environment variables or credentials are copied into evidence.
    tool_records = []
    for tool in tools:
        record = {k: tool[k] for k in ("id", "version", "commit", "nativeOffline", "settings")}
        record["executable"] = bundle.copy(f"provenance/{tool['id']}-executable", tool["executable"], MAX_GRAPH_BYTES)
        record["environment"] = bundle.copy(f"provenance/{tool['id']}-environment.json", tool["environment"])
        env = _json(external(tool["environment"])[1])
        for dependency in env["files"]:
            path, data = external(dependency, MAX_GRAPH_BYTES)
            bundle.inputs[path] = (sha(data), MAX_GRAPH_BYTES)
        record["normalizer"] = normalizer_ref
        if "releaseVerification" in tool:
            record["releaseVerification"] = bundle.copy(f"provenance/{tool['id']}-release.json", tool["releaseVerification"])
        tool_records.append(record)

    def execute(argv: tuple[str, ...], relative: str, cwd: Path, seconds: int) -> tuple[dict, object]:
        path = bundle.root / relative
        result = runner(argv, cwd=cwd, timeout_seconds=seconds,
                        stdout_path=path.with_suffix(".stdout"), stderr_path=path.with_suffix(".stderr"))
        out = read_bounded(path.with_suffix(".stdout"), MAX_JSON_BYTES)
        err = read_bounded(path.with_suffix(".stderr"), MAX_JSON_BYTES)
        # run_bounded owns bounded raw capture; write no-clobber JSON sidecar.
        def ref(p, data):
            return dict(path=p.relative_to(bundle.root).as_posix(), sha256=sha(data))
        capture = dict(argv=list(result.argv), exit_code=result.exit_code, timed_out=result.timed_out,
                       output_limited=result.output_limited, wall_ms=result.wall_ms,
                       stdout_bytes=len(out), stderr_bytes=len(err),
                       stdout=ref(path.with_suffix(".stdout"), out), stderr=ref(path.with_suffix(".stderr"), err))
        bundle.total += len(out) + len(err)
        if bundle.total > 2 * 1024**3:
            raise ValueError("capture bundle exceeds two GiB bound")
        return bundle.put(relative, capture), result

    def source_state(phase: str):
        for repo in registration["repositories"]:
            root = Path(roots[repo["id"]])
            for name, args in (("head", ("rev-parse", "HEAD")), ("status", ("status", "--porcelain", "--untracked-files=all"))):
                _, result = execute(("git", "-C", str(root), *args), f"source-checks/{key(repo['id'])}-{phase}-{name}.json", root, 15)
                if result.exit_code or result.timed_out or result.output_limited:
                    raise ValueError("bounded source verification failed")
                if result.stdout.strip() != (repo["commit"] if name == "head" else ""):
                    raise ValueError("source pin or cleanliness changed")
        _verify_sources(registration, roots)

    source_state("before")
    captured_graphs = []
    graph_paths = {}
    for entry in graph_records:
        prefix = f"graphs/{key(entry['repository'])}-{entry['tool']}"
        copied = {k: entry[k] for k in ("repository", "tool", "sourceCommit")}
        for field in ("artifact", "sourceInventory"):
            copied[field] = bundle.copy(prefix + "-" + field + ".json", entry[field], MAX_GRAPH_BYTES if field == "artifact" else MAX_JSON_BYTES)
        copied["originalBuildCapture"] = bundle.copy(prefix + "-original-build.json", entry["buildCapture"])
        build = _json(external(entry["buildCapture"])[1])
        for stream in ("stdout", "stderr"):
            build[stream] = bundle.copy(prefix + "-build." + stream, build[stream])
        copied["buildCapture"] = bundle.put(prefix + "-build.json", build)
        copied["sourceCleanBeforeAndAfter"] = True  # published only after final verification below
        captured_graphs.append(copied)
        graph_paths[entry["repository"], entry["tool"]] = bundle.root / copied["artifact"]["path"]
    observations, reviews = [], []
    # Fixed order is declared; no adaptive retries, free follow-ups or exclusions.
    for task in sorted(registration["tasks"], key=lambda t: t["id"]):
        for arm in ARMS:
            for tool in sorted(tools, key=lambda t: t["id"]):
                name = tool["id"]
                argv = command(plan, tool, arm, task["question"], graph_paths[task["repository"], name])
                timed_argv = launch(plan, argv, bundle.root)
                prefix = f"queries/{key(task['id'])}/{name}-{arm}"
                capture, result = execute(timed_argv, prefix + ".json", bundle.root, timeout)
                # Normalize original bytes, never a replacement-decoded subprocess string.
                raw = read_bounded(bundle.root / (prefix + ".stdout"), MAX_JSON_BYTES)
                try:
                    answer = normalize(raw, name, graph_indexes[task["repository"], name],
                                       task=task["id"], arm=arm, failed=result.exit_code != 0,
                                       limited=result.timed_out or result.output_limited)
                except (UnicodeError, ValueError) as error:
                    answer = dict(schema=NORMALIZED_SCHEMA, task=task["id"], tool=name, arm=arm,
                                  disposition="limit" if result.timed_out or result.output_limited else "error",
                                  rankingKind="unavailable", admission=None, ranked=[], claims=[],
                                  normalizationComplete=False, reviewQueue=[dict(kind="parser_rejection", reason=str(error))])
                normalized_ref = bundle.put(prefix + "-normalized.json", answer)
                peak = rss(read_bounded(bundle.root / (prefix + ".stderr"), MAX_JSON_BYTES), style)
                observations.append(dict(task=task["id"], tool=name, arm=arm, captures=[capture],
                                         normalized=normalized_ref, condition="fresh_cli", peakRssBytes=peak))
                reviews.append(dict(task=task["id"], tool=name, arm=arm,
                                    normalizedSha256=normalized_ref["sha256"], candidates=[],
                                    claims=[dict(id=c["id"], verdict="unjudged", factId=None,
                                                 reason="Pending independent source-aware review of actual printed assertion.") for c in answer["claims"]],
                                    independentReview=dict(PENDING)))
    source_state("after")
    bundle.recheck()
    run = bundle.put("run.json", dict(schema=RUN_SCHEMA, registrationSha256=registration_ref["sha256"],
                     startedAt=started, completedAt=datetime.now(timezone.utc).isoformat(), tools=tool_records,
                     graphs=captured_graphs, arms=arms, observations=observations,
                     measurementExecutable=measurement_ref, environmentExecutable=environment_launcher_ref,
                     childEnvironment=isolated_environment,
                     collectionOrder="task_id_then_default_normalized_then_tool_id; no retries or followups",
                     measurementScope="fresh isolated CLI processes, unflushed OS cache; time wrapper RSS; no cold-cache claim"))
    reviewed = bundle.put("reviews-pending.json", dict(schema=REVIEW_SCHEMA, runSha256=run["sha256"], observations=reviews))
    manifest = dict(schema=MANIFEST_SCHEMA, registration=registration_ref, run=run, reviews=reviewed,
                    sourceRoots=roots, graphQualification=[], exposureEvidence=copied_exposure)
    bundle.put("review-input.json", manifest)
    status = review_main(["--score", str(bundle.root / "review-input.json"), "--output", str(bundle.root / "report-provisional.json")])
    if status:
        raise ValueError("captured evidence failed provisional report generation; inspect incomplete bundle")
    bundle.put("COMPLETE.json", dict(status="captured_pending_independent_output_review", qualificationEstablished=False,
                                    note="COMPLETE confirms capture publication only. --check must still reject unqualified claims."))
    return dict(bundle=str(bundle.root), observations=len(observations), report=str(bundle.root / "report-provisional.json"),
                qualificationEstablished=False)


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    actions = parser.add_mutually_exclusive_group(required=True)
    actions.add_argument("--capture", type=Path, help="predeclared capture input; executes both explicit CLIs")
    actions.add_argument("--self-test", action="store_true")
    parser.add_argument("--output", type=Path, help="new external artifact directory")
    args = parser.parse_args(argv)
    try:
        if args.self_test:
            import unittest
            suite = unittest.defaultTestLoader.loadTestsFromName("benchmarks.agent_query.tests.test_broad_query_capture")
            return 0 if unittest.TextTestRunner(verbosity=1).run(suite).wasSuccessful() else 1
        if args.output is None:
            parser.error("--capture requires --output")
        print(json.dumps(collect(_json(read_bounded(args.capture, MAX_JSON_BYTES)), args.output)))
        return 0
    except (OSError, ValueError, TypeError, KeyError, IndexError, RecursionError) as error:
        print(f"broad-query capture rejected: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
