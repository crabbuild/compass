"""Source-frozen panels over one immutable protocol2 union capture.

No runs/normalizations are projected or rewritten, and no queries are executed.
Shared observations are scored once; panel statistics reuse their exact rows.
This internal integration does not authenticate source or independent reviews.
"""
from pathlib import Path

from benchmarks.agent_query import broad_query_panels as legacy
from benchmarks.agent_query import broad_query_review as old
from benchmarks.agent_query.broad_query_lineage_v2 import _reference

DEFINITION_SCHEMA = "compass.broad-query-panels/2"
BINDING_SCHEMA = "compass.broad-query-panels-binding/2"
ROLES = ("original", "lookup")


def _panel_reference(value, external):
    if not external:
        return _reference(value)
    if type(value) is not dict or value.keys() != {"path", "sha256"}:
        raise ValueError("original panel reference needs exactly path and sha256")
    old._digest(value["sha256"])
    if not Path(old._text(value["path"], "absolute original panel input")).is_absolute():
        raise ValueError("original panel input must be absolute")
    return value


def read_panels(registration, inputs, read, actors, *, external=False):
    """Verify a finite panel definition and both source registrations.

    read is the shared original or retained bounded reader. Definition bytes
    are committed by the union source registration before any output exists.
    Role registrations are bound by digest, avoiding a circular union hash.
    """
    binding = registration.get("comparisonPanels")
    if binding is None and inputs is None:
        return None
    if (type(binding) is not dict or binding.keys() != {"schema", "definitionSha256"}
            or binding["schema"] != BINDING_SCHEMA
            or type(inputs) is not dict or inputs.keys() != {"definition", "registrations"}):
        raise ValueError("panels must match their source-frozen union binding")
    definition_ref = _panel_reference(inputs["definition"], external)
    if definition_ref["sha256"] != old._digest(binding["definitionSha256"]):
        raise ValueError("panel definition differs from source-frozen digest")
    definition = old._json(read(definition_ref))
    if (definition.keys() != {"schema", "frozenAt", "actorSha256", "registrations", "lookupPairs"}
            or definition["schema"] != DEFINITION_SCHEMA or definition["actorSha256"] != actors):
        raise ValueError("unsupported panel definition or foreign actors")
    definitions = legacy._roles(definition["registrations"], "panel registration digests")
    references = legacy._roles(inputs["registrations"], "panel registration references")
    registrations = {}
    for role in ROLES:
        reference = _panel_reference(references[role], external)
        if reference["sha256"] != old._digest(definitions[role]):
            raise ValueError("panel registration differs from frozen definition")
        registrations[role] = old.validate_registration(old._json(read(reference)))
        if "comparisonPanels" in registrations[role]:
            raise ValueError("transitive panel definitions are unsupported")
    # Reuse the existing source-control validation, not its run projection or
    # report arithmetic. Only the in-memory helper's schema tag is adapted.
    tasks, common = legacy.validate_panels(dict(definition, schema=legacy.DEFINITION_SCHEMA), registrations)
    union_tasks = {task["id"]: task for task in registration["tasks"]}
    expected = {**tasks["original"], **tasks["lookup"]}
    if union_tasks != expected:
        raise ValueError("union source tasks must preserve every original and lookup task exactly")
    if old._timestamp(definition["frozenAt"]) > old._timestamp(registration["frozenAt"]):
        raise ValueError("union source registration predates its panel definition")
    union_profile, union_selection = legacy._calibration_binding(registration)
    for role in ROLES:
        panel = registrations[role]
        for field in ("schema", "designation", "qualificationEligible", "repositories", "arms", "capturePlan",
                "gatePolicy", "graphQualificationRepositories", "exposureLineage"):
            if panel.get(field) != registration.get(field):
                raise ValueError("union and panel have different frozen " + field)
        profile, selection = legacy._calibration_binding(panel)
        if profile != union_profile or selection != {key: value for key, value in union_selection.items() if key in tasks[role]}:
            raise ValueError("panel calibration is not the exact frozen union restriction")
        if (old._review_failures(panel.get("independentReview"), role + " source review")
                or old.calibration_review_failures(panel)
                or ("developmentCalibration" not in panel and panel.get("sourceReview", {}).get("blindToOutputs") is not True)
                or panel.get("independentReview", {}).get("reviewer") == panel.get("sourceReview", {}).get("author")):
            raise ValueError("panel source review is incomplete, exposed or not independent")
    return dict(definition=definition, registrations=registrations, tasks=tasks, common=common)


def retain_panels(bundle, inputs):
    if inputs is None:
        return None
    return dict(definition=bundle.copy("panels/definition.json", inputs["definition"]),
        registrations={role: bundle.copy(f"panels/{role}-registration.json", inputs["registrations"][role])
            for role in ROLES})


def finish_panels(panel, manifest, registration, run, artifacts, started, scored, costs,
        row_failures, reviewers, union_report):
    """Apply every existing gate separately; cost each union execution once.

    Called only after the union lineage has been fully consumed and scored.
    The caller releases union graph/source indexes first; each panel's shared
    metadata check is sequential and its temporary indexes are released before
    the next. No observation/output normalization or native query is repeated.
    """
    reports, failures = {}, []
    for role in ROLES:
        source = panel["registrations"][role]
        old.verify_calibration_evidence(source, manifest.get("calibrationEvidence", {}), artifacts.read)
        state = old._prepare_assessment(manifest, source, run, artifacts, started,
            actors=run["protocolBinding"]["actorSha256"])
        state["graph_indexes"].clear()
        state["sources"].clear()
        if run.get("captureInputInvocation") is None:
            state["failures"].append("capture-input invocation file was not retained; object snapshots are diagnostic only")
        keys = {key for key in scored if key[0] in panel["tasks"][role]}
        for key in sorted(keys):
            state["failures"].extend(row_failures[key])
        selected = {key: scored[key] for key in sorted(keys)}
        reports[role] = old._finish_assessment(manifest, source, artifacts, started, state, selected,
            [cost for cost in costs if cost["task"] in panel["tasks"][role]],
            sorted({reviewers[key] for key in keys}), protocol2=True)
        if not reports[role]["eligibleForSuperiorityClaim"]:
            failures.extend(f"{role}: {failure}" for failure in reports[role]["failures"])
        del state
    if not union_report["eligibleForSuperiorityClaim"]:
        failures.extend("union: " + failure for failure in union_report["failures"])
    build_costs = []
    for graph in run["graphs"]:
        record = artifacts.document(graph["buildCapture"])
        build_costs.append(dict(repository=graph["repository"], tool=graph["tool"], workflowWallMs=record["wall_ms"],
            workflowBytes=record["stdout_bytes"] + record["stderr_bytes"],
            failed=bool(record["exit_code"] or record["timed_out"] or record["output_limited"])))
    return dict(schema="compass.broad-query-panel-assessment/2", eligibleForSuperiorityClaim=not failures,
        failures=sorted(set(failures)), reports=reports,
        panelEligibility={**{role: reports[role]["eligibleForSuperiorityClaim"] for role in ROLES},
            "union": union_report["eligibleForSuperiorityClaim"]},
        panelComparisons={**{role: reports[role]["comparisons"] for role in ROLES}, "union": union_report["comparisons"]},
        distinctQuestions=len(registration["tasks"]), sharedQuestions=len(panel["common"]), distinctObservations=len(costs),
        costs=sorted(costs, key=lambda row: (row["task"], row["tool"], row["arm"])),
        buildCosts=sorted(build_costs, key=lambda row: (row["repository"], row["tool"])),
        costSummaries={tool: {arm: old._cost_summary([row for row in costs if row["tool"] == tool and row["arm"] == arm])
            for arm in old.ARMS} for tool in old.TOOLS},
        verificationBoundary="One immutable union lineage and scoring pass. Each source-frozen panel retains all gates; shared native captures/builds charged once. Reviewer authenticity and source semantics remain separate.")
