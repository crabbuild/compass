"""Require both frozen comparison panels and charge shared observations once.

This developer-side binder executes no product or model. It recomputes both
canonical review reports; it cannot authenticate reviewers or source meaning.
"""
import argparse
from copy import deepcopy
import json
from pathlib import Path
import sys

from benchmarks.agent_query import broad_query_review as canonical
from benchmarks.agent_query.broad_query_oracle import publish_exclusive
from benchmarks.agent_query.broad_query_review import (
    ARMS, MAX_JSON_BYTES, MAX_TASKS, TOOLS, Artifacts, _array, _cost_summary,
    _json, _object, _text, _timestamp, read_bounded, recompute_report, sha,
    validate_registration,
)

DEFINITION_SCHEMA = "compass.broad-query-panels/1"
INPUT_SCHEMA = "compass.broad-query-panel-review-input/1"
REPORT_SCHEMA = "compass.broad-query-panel-review/1"
ROLES = ("original", "lookup")
TASK_FIELDS = ("id", "repository", "cluster", "kind", "question", "expected",
               "relevant", "facts", "rationale", "split", "mechanisms")


def _assessors():
    return dict(panel=sha(read_bounded(Path(__file__), MAX_JSON_BYTES)),
                canonical=sha(read_bounded(Path(canonical.__file__), MAX_JSON_BYTES)))


def _semantic_task(task):
    return {key: task[key] for key in TASK_FIELDS if key in task}


def _content(value):
    """Artifact aliases may differ; their actual, checked bytes may not."""
    if isinstance(value, dict):
        if set(value) == {"path", "sha256"}:
            return {"sha256": value["sha256"]}
        return {key: _content(item) for key, item in value.items()}
    if isinstance(value, list):
        return [_content(item) for item in value]
    return value


def _roles(raw, where):
    raw = _object(raw, where)
    if set(raw) != set(ROLES):
        raise ValueError(f"{where} requires exactly original and lookup panels")
    return raw


def validate_panels(definition, registrations):
    """Bind the role correction without discarding or duplicating source units."""
    if definition.get("schema") != DEFINITION_SCHEMA:
        raise ValueError("unsupported comparison panel definition schema")
    stamp = _timestamp(definition.get("frozenAt"))
    _roles(registrations, "registrations")
    for registration in registrations.values():
        validate_registration(registration)
        if _timestamp(registration["frozenAt"]) > stamp:
            raise ValueError("panel definition predates a source registration")
    a, b = (registrations[role] for role in ROLES)
    if a["repositories"] != b["repositories"]:
        raise ValueError("panels have different source inventories or repository order")
    for field in ("arms", "gatePolicy", "metricCohorts", "graphQualificationRepositories", "capturePlan", "exposureLineage"):
        # Cohort IDs are checked below after applying the declared task mapping.
        if field != "metricCohorts" and a.get(field) != b.get(field):
            raise ValueError(f"panels have different frozen {field}")
    tasks = {role: {t["id"]: t for t in registrations[role]["tasks"]} for role in ROLES}
    pairs = _array(definition.get("lookupPairs"), "lookup task pairs", MAX_TASKS, 1)
    mapping = {}
    for pair in pairs:
        pair = _object(pair, "lookup task pair")
        old = _text(pair.get("original"), "original task ID", 128)
        new = _text(pair.get("lookup"), "lookup task ID", 128)
        if old in mapping or new in mapping.values() or old == new:
            raise ValueError("lookup pairs must have distinct, unique task IDs")
        mapping[old] = new
    common = set(tasks["original"]) & set(tasks["lookup"])
    if set(tasks["original"]) - common != set(mapping) or set(tasks["lookup"]) - common != set(mapping.values()):
        raise ValueError("lookup pairs do not preserve the complete original panel")
    for key in common:
        if _semantic_task(tasks["original"][key]) != _semantic_task(tasks["lookup"][key]):
            raise ValueError("shared task question, fact, cluster or disposition changed")
    if not any(tasks["original"][key]["kind"] == "broad" for key in common):
        raise ValueError("the original broad tasks must be preserved")
    for old, new in mapping.items():
        original, lookup = tasks["original"][old], tasks["lookup"][new]
        if original["kind"] != "exact" or lookup["kind"] != "exact":
            raise ValueError("only named exact tasks may gain lookup variants")
        for field in ("repository", "cluster", "relevant", "expected", "split", "mechanisms"):
            if original.get(field) != lookup.get(field):
                raise ValueError("lookup variants changed source identity or source group")
        if len(lookup["relevant"]) != 1 or lookup["relevant"][0]["grade"] != 3 or len(lookup["facts"]) != 1:
            raise ValueError("lookup controls require one declaration identity and fact")
        identity = lookup["relevant"][0]["identity"]
        symbol = (identity["owner"] + "." if identity["owner"] else "") + identity["symbol"]
        if lookup["question"] != f"Locate the declaration of {symbol} in {identity['file']}.":
            raise ValueError("lookup control does not request canonical declaration lookup")
        fact = lookup["facts"][0]
        if (fact["identity"] != identity or fact["essential"] is not True
                or fact.get("relation") is not None
                or fact.get("statement") != "Declaration identity at the frozen source anchor."
                or fact["source"]["file"] != identity["file"]
                or fact["source"]["line"] != identity["startLine"]):
            raise ValueError("lookup control fact is not its declaration identity")
    for metric in set(a.get("metricCohorts", {})) | set(b.get("metricCohorts", {})):
        first = a.get("metricCohorts", {}).get(metric)
        second = b.get("metricCohorts", {}).get(metric)
        if not isinstance(first, dict) or not isinstance(second, dict):
            raise ValueError("both panels must preserve every frozen metric cohort")
        expected = {mapping.get(key, key) for key in first["taskIds"]}
        if expected != set(second["taskIds"]) or {k: v for k, v in first.items() if k != "taskIds"} != {k: v for k, v in second.items() if k != "taskIds"}:
            raise ValueError("lookup panel changed the original metric cohort")
    return tasks, common


def _indexed(rows, where):
    result = {}
    for row in _array(rows, where, MAX_TASKS * 4):
        row = _object(row, where)
        key = (row.get("task"), row.get("tool"), row.get("arm"))
        if key in result:
            raise ValueError(f"duplicate {where}")
        result[key] = row
    return result


def project_capture(manifest, definition_ref, root):
    """Project one union capture in place; preserve its isolated environment.

    Projection emits no accepted judgments or eligibility. The final receipt is
    published only after both complete review inputs have been written.
    """
    if manifest.get("schema") != canonical.MANIFEST_SCHEMA:
        raise ValueError("unsupported union review-input schema")
    artifacts = Artifacts(root)
    definition = artifacts.document(definition_ref)
    if definition.get("schema") != DEFINITION_SCHEMA:
        raise ValueError("unsupported comparison panel definition schema")
    assessor_digests = _assessors()
    if definition.get("assessorSha256") != assessor_digests:
        raise ValueError("comparison panel assessors differ from the frozen definition")
    definitions = _roles(definition.get("registrations"), "panel definitions")
    registrations = {role: artifacts.document(definitions[role]) for role in ROLES}
    tasks, _ = validate_panels(definition, registrations)
    registration = validate_registration(artifacts.document(manifest.get("registration")))
    run = artifacts.document(manifest.get("run"))
    reviewed = artifacts.document(manifest.get("reviews"))
    if (run.get("schema") != canonical.RUN_SCHEMA or reviewed.get("schema") != canonical.REVIEW_SCHEMA
            or run.get("registrationSha256") != manifest["registration"]["sha256"]
            or reviewed.get("runSha256") != manifest["run"]["sha256"]):
        raise ValueError("union capture/reviews lack their exact registration/run binding")
    if _timestamp(definition["frozenAt"]) >= _timestamp(run.get("startedAt")):
        raise ValueError("panel binding was not frozen before collection")
    if _timestamp(registration["frozenAt"]) >= _timestamp(run["startedAt"]):
        raise ValueError("union registration was not frozen before collection")
    expected_tasks = {**tasks["original"], **tasks["lookup"]}
    union_tasks = {t["id"]: t for t in registration["tasks"]}
    if (set(union_tasks) != set(expected_tasks)
            or any(_semantic_task(union_tasks[key]) != _semantic_task(expected_tasks[key]) for key in union_tasks)):
        raise ValueError("union capture dropped or changed a required question/fact")
    for role in ROLES:
        for field in ("repositories", "arms", "capturePlan", "exposureLineage"):
            if registrations[role].get(field) != registration.get(field):
                raise ValueError(f"union and panel have different frozen {field}")
    observations = _indexed(run.get("observations"), "union observations")
    judgments = _indexed(reviewed.get("observations"), "union judgments")
    expected = {(key, tool, arm) for key in union_tasks for tool in TOOLS for arm in ARMS}
    if set(observations) != expected or set(judgments) != expected:
        raise ValueError("union capture/review matrix is incomplete")
    prepared = {}
    receipt = dict(schema="compass.broad-query-panel-projection/1", definition=definition_ref,
                   unionInputCanonicalSha256=sha(json.dumps(manifest, sort_keys=True, allow_nan=False).encode()),
                   unionRun=manifest["run"], inputs={}, qualificationEstablished=False)
    for role in ROLES:
        selected = sorted(key for key in expected if key[0] in tasks[role])
        projected = deepcopy(run)
        projected["registrationSha256"] = definitions[role]["sha256"]
        projected["observations"] = [observations[key] for key in selected]
        run_name = f"panels-{role}-run.json"
        run_bytes = (json.dumps(projected, sort_keys=True, allow_nan=False) + "\n").encode()
        prepared[run_name] = run_bytes
        run_ref = dict(path=run_name, sha256=sha(run_bytes))
        review = deepcopy(reviewed)
        review["runSha256"] = run_ref["sha256"]
        review["observations"] = [judgments[key] for key in selected]
        review_name = f"panels-{role}-reviews.json"
        review_bytes = (json.dumps(review, sort_keys=True, allow_nan=False) + "\n").encode()
        prepared[review_name] = review_bytes
        source = deepcopy(manifest)
        source.update(registration=definitions[role], run=run_ref,
                      reviews=dict(path=review_name, sha256=sha(review_bytes)))
        name = f"panels-{role}-input.json"
        data = (json.dumps(source, sort_keys=True, allow_nan=False) + "\n").encode()
        prepared[name] = data
        receipt["inputs"][role] = dict(path=name, sha256=sha(data))
    prepared["panels-projection.json"] = (json.dumps(receipt, sort_keys=True, allow_nan=False) + "\n").encode()
    if any(len(data) > MAX_JSON_BYTES for data in prepared.values()):
        raise ValueError("projected artifact exceeds bounded JSON size")
    if any((Path(root) / name).exists() for name in prepared):
        raise ValueError("projection outputs already exist; originals will not be overwritten")
    artifacts.recheck()
    if _assessors() != assessor_digests:
        raise ValueError("comparison panel assessor changed during projection")
    for name, data in prepared.items():
        publish_exclusive(Path(root) / name, data)
    return receipt


def assess_panels(manifest, root):
    if manifest.get("schema") != INPUT_SCHEMA:
        raise ValueError("unsupported comparison panel input schema")
    artifacts = Artifacts(root)
    definition = artifacts.document(manifest.get("definition"))
    if definition.get("schema") != DEFINITION_SCHEMA:
        raise ValueError("unsupported comparison panel definition schema")
    assessor_digests = _assessors()
    if definition.get("assessorSha256") != assessor_digests:
        raise ValueError("comparison panel assessors differ from the frozen definition")
    defined = _roles(definition.get("registrations"), "panel definitions")
    registrations = {role: artifacts.document(defined[role]) for role in ROLES}
    tasks, common = validate_panels(definition, registrations)
    reports = _roles(manifest.get("reports"), "required reports")
    assessments, runs, judgments, costs = {}, {}, {}, {}
    readers = []
    input_digests = {}
    failures = []
    for role in ROLES:
        report = artifacts.document(reports[role])
        # Never accept an asserted eligible boolean or stale stored arithmetic.
        assessments[role] = recompute_report(report)
        data = read_bounded(Path(report["inputPath"]), MAX_JSON_BYTES)
        if sha(data) != report["inputSha256"]:
            raise ValueError("panel input changed during recomputation")
        input_digests[Path(report["inputPath"])] = report["inputSha256"]
        source = _json(data)
        reader = Artifacts(Path(report["inputPath"]).parent)
        readers.append(reader)
        registration = reader.document(source["registration"])
        if registration != registrations[role] or source["registration"]["sha256"] != defined[role]["sha256"]:
            raise ValueError("report source registration differs from frozen panel definition")
        run = reader.document(source["run"])
        if _timestamp(definition["frozenAt"]) >= _timestamp(run["startedAt"]):
            raise ValueError("panel binding was not frozen before collection")
        runs[role] = run
        judgments[role] = _indexed(reader.document(source["reviews"])["observations"], "review observations")
        costs[role] = _indexed(assessments[role]["costs"], "cost observations")
        if not assessments[role]["eligibleForSuperiorityClaim"]:
            failures.extend(f"{role}: {failure}" for failure in
                            (assessments[role]["failures"] or ["canonical panel is ineligible"]))
    for field in ("tools", "graphs", "arms", "measurementExecutable", "environmentExecutable", "childEnvironment"):
        if _content(runs["original"].get(field)) != _content(runs["lookup"].get(field)):
            raise ValueError(f"panels used different captured {field}")
    observations = {role: _indexed(runs[role]["observations"], "captured observations") for role in ROLES}
    union_costs = {}
    for role in ROLES:
        expected = {(key, tool, arm) for key in tasks[role] for tool in TOOLS for arm in ARMS}
        if set(observations[role]) != expected or set(costs[role]) != expected or set(judgments[role]) != expected:
            raise ValueError("panel has an incomplete observation/cost/review matrix")
        for key in sorted(expected):
            if key in union_costs:
                if (_content(observations["original"][key]) != _content(observations["lookup"][key])
                        or _content(judgments["original"][key]) != _content(judgments["lookup"][key])
                        or costs["original"][key] != costs["lookup"][key]):
                    raise ValueError("shared observation, judgment or cost was not reused identically")
            else:
                union_costs[key] = costs[role][key]
    build_costs = []
    for graph in runs["original"]["graphs"]:
        capture = readers[0].document(graph["buildCapture"])
        build_costs.append(dict(repository=graph["repository"], tool=graph["tool"],
                               workflowWallMs=capture["wall_ms"],
                               workflowBytes=capture["stdout_bytes"] + capture["stderr_bytes"],
                               failed=bool(capture["exit_code"] or capture["timed_out"] or capture["output_limited"])))
    for reader in readers:
        reader.recheck()
    for path, expected_digest in input_digests.items():
        if sha(read_bounded(path, MAX_JSON_BYTES)) != expected_digest:
            raise ValueError("panel input changed during aggregate assessment")
    if assessor_digests != _assessors():
        raise ValueError("comparison panel assessor changed during aggregate assessment")
    artifacts.recheck()
    rows = [union_costs[key] for key in sorted(union_costs)]
    return dict(eligibleForSuperiorityClaim=not failures, failures=sorted(set(failures)),
                panelEligibility={role: assessments[role]["eligibleForSuperiorityClaim"] for role in ROLES},
                panelComparisons={role: assessments[role]["comparisons"] for role in ROLES},
                distinctQuestions=len(set(tasks["original"]) | set(tasks["lookup"])),
                sharedQuestions=len(common), distinctObservations=len(rows), costs=rows,
                buildCosts=sorted(build_costs, key=lambda row: (row["repository"], row["tool"])),
                costSummaries={tool: {arm: _cost_summary([row for row in rows if row["tool"] == tool and row["arm"] == arm]) for arm in ARMS} for tool in TOOLS},
                verificationBoundary="Both canonical reports recomputed; shared captures, judgments and costs bound. Reviewer authenticity, source meaning and statistical independence remain outside programmatic verification.")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    action = parser.add_mutually_exclusive_group(required=True)
    action.add_argument("--score", type=Path)
    action.add_argument("--check", type=Path)
    action.add_argument("--project", type=Path, help="union review input; publish both projections in the same artifact root")
    parser.add_argument("--definition", type=Path, help="contained pre-capture panel definition; required for --project")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        if args.project:
            if args.definition is None:
                parser.error("--project requires --definition")
            root = args.project.resolve().parent
            path = args.definition.resolve(strict=True)
            if not path.is_relative_to(root):
                raise ValueError("panel definition must be contained in the union artifact root")
            data = read_bounded(path, MAX_JSON_BYTES)
            receipt = project_capture(_json(read_bounded(args.project, MAX_JSON_BYTES)),
                                      dict(path=path.relative_to(root).as_posix(), sha256=sha(data)), root)
            print(json.dumps(receipt))
            return 0
        if args.score:
            if args.output is None:
                parser.error("--score requires --output")
            data = read_bounded(args.score, MAX_JSON_BYTES)
            assessment = assess_panels(_json(data), args.score.parent)
            report = dict(schema=REPORT_SCHEMA, inputPath=str(args.score.resolve()), inputSha256=sha(data),
                          assessorSha256=sha(read_bounded(Path(__file__), MAX_JSON_BYTES)), assessment=assessment)
            encoded = (json.dumps(report, indent=2, sort_keys=True, allow_nan=False) + "\n").encode()
            if len(encoded) > MAX_JSON_BYTES:
                raise ValueError("panel report exceeds bounded JSON limit")
            args.output.parent.mkdir(parents=True, exist_ok=True)
            publish_exclusive(args.output, encoded)
            print(json.dumps(dict(report=str(args.output), eligibleForSuperiorityClaim=assessment["eligibleForSuperiorityClaim"])))
            return 0
        report = _json(read_bounded(args.check, MAX_JSON_BYTES))
        if report.get("schema") != REPORT_SCHEMA or report.get("assessorSha256") != sha(read_bounded(Path(__file__), MAX_JSON_BYTES)):
            raise ValueError("unsupported or stale comparison panel report")
        path = Path(_text(report.get("inputPath"), "input path", 4096))
        data = read_bounded(path, MAX_JSON_BYTES)
        if sha(data) != report.get("inputSha256"):
            raise ValueError("comparison panel input changed")
        assessment = assess_panels(_json(data), path.parent)
        if assessment != report.get("assessment"):
            raise ValueError("comparison panel metrics or eligibility were fabricated or stale")
        print(json.dumps(dict(eligibleForSuperiorityClaim=assessment["eligibleForSuperiorityClaim"], failures=assessment["failures"])))
        return 0 if assessment["eligibleForSuperiorityClaim"] else 1
    except (OSError, ValueError, TypeError, KeyError, IndexError, RecursionError) as error:
        print(f"comparison panel review rejected: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
