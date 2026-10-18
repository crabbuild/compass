"""Check one retained protocol2 native execution and its measured cost.

The file-backed assessor must first verify the frozen tools, graph, sources and
actors. This layer checks their exact command against the original capture;
it does not authenticate a subprocess or establish source/answer quality.
"""
from pathlib import Path

from benchmarks.agent_query import broad_query_capture as capture
from benchmarks.agent_query import broad_query_review as old
from benchmarks.agent_query.broad_query_lineage_v2 import require_schema
from benchmarks.agent_query.broad_query_protocol import MAX_RAW_BYTES


def validate_execution(observation, task, plan, graph_entry, root):
    """Return cost/failure records without dropping failed or oversized rows.

    observation is a BoundObservation from the single lineage traversal. This
    function performs no I/O, execution, source scanning or output clipping.
    root must be the auditor's own artifact root, not captured metadata.
    """
    require_schema(plan, "capture-plan")
    task_id, tool_id, arm_id = observation.key
    if task.get("id") != task_id:
        raise ValueError("execution task differs from frozen lineage")
    if (graph_entry.get("repository"), graph_entry.get("tool")) != (task.get("repository"), tool_id):
        raise ValueError("execution graph differs from frozen task and native tool")
    graph_path = old._relative(old._object(graph_entry.get("artifact"), "graph reference").get("path"))
    tools = old._array(plan.get("tools"), "frozen tools", 2, 2)
    tool_ids = [old._choice(old._object(tool, "frozen tool").get("id"), old.TOOLS, "native tool") for tool in tools]
    if set(tool_ids) != set(old.TOOLS):
        raise ValueError("execution needs both unique frozen tools")
    tool = next(tool for tool in tools if tool["id"] == tool_id)
    old.query_exit_contract(tool)
    arms = old._array(plan.get("arms"), "frozen arms", 2, 2)
    arm_ids = [old._choice(old._object(arm, "frozen arm").get("id"), old.ARMS, "cost arm") for arm in arms]
    if set(arm_ids) != set(old.ARMS):
        raise ValueError("execution needs both unique frozen arms")
    arm_map = {arm["id"]: arm for arm in arms}
    if arm_map["default"] != dict(id="default"):
        raise ValueError("default arm must use the unchanged native default command")
    normalized = arm_map["normalized"]
    if (not normalized.keys() <= {"id", "budgetBytes", "measure", "overflow", "compassView", "configuredComparison"}
            or normalized.get("measure") != "utf8_bytes" or normalized.get("overflow") != "fail"):
        raise ValueError("normalized arm needs the frozen byte budget and overflow=fail")
    budget = old._integer(normalized.get("budgetBytes"), "normalized byte budget", 128, 1024**2)
    view = old._choice(normalized.get("compassView", "default"), ("default", "ranked"), "Compass view")
    if "configuredComparison" in normalized:
        old._boolean(normalized["configuredComparison"], "configured comparison")
    if view == "ranked" and normalized.get("configuredComparison") is not True:
        raise ValueError("ranked view must be frozen as a configured comparison")
    old._integer(plan.get("timeoutSeconds"), "frozen timeout seconds", 1, 300)
    measurement = old._object(plan.get("measurement"), "frozen measurement")
    style = old._choice(measurement.get("style"), ("darwin-l", "gnu-v"), "RSS instrumentation")
    question = old._text(task.get("question"), "frozen question", 16384)
    expected = capture.launch(plan, capture.command(plan, tool, arm_id,
        question, Path(root) / graph_path), Path(root))
    recorded = observation.capture
    argv = old._array(recorded.get("argv"), "captured argv", 128, 1)
    for argument in argv:
        old._text(argument, "captured argument", 16384)
    if tuple(argv) != expected:
        raise ValueError("captured argv differs from the exact frozen native command")
    for name, data in (("stdout", observation.stdout), ("stderr", observation.stderr)):
        if type(data) is not bytes or len(data) > MAX_RAW_BYTES:
            raise ValueError("execution needs original bounded native stream bytes")
        if old._integer(recorded.get(name + "_bytes"), "captured stream size", 0, MAX_RAW_BYTES) != len(data):
            raise ValueError("native stream byte count differs from original bytes")
    elapsed = old._integer(recorded.get("wall_ms"), "capture latency", 0, 86_400_000)
    timed_out = old._boolean(recorded.get("timed_out"), "capture timeout")
    limited = old._boolean(recorded.get("output_limited"), "capture output limit")
    outcome = old.query_outcome(tool, recorded.get("exit_code"), observation.stdout,
        timed_out=timed_out, limited=limited)
    peak = capture.rss(observation.stderr, style)
    if peak is not None:
        old._integer(peak, "measured RSS bytes", 1)
    reported_peak = observation.captured.get("peakRssBytes")
    if reported_peak is not None:
        old._integer(reported_peak, "recorded RSS bytes", 1)
    if reported_peak != peak:
        raise ValueError("recorded peak RSS differs from original measurement stderr")
    if observation.captured.get("condition") != "fresh_cli":
        raise ValueError("protocol2 collection requires the frozen fresh CLI condition")
    overflow = arm_id == "normalized" and len(observation.stdout) > budget
    incompatible = outcome == "no_match" and observation.reviewed.get("disposition") not in ("abstain", "error", "limit")
    failed = outcome == "failure" or overflow or incompatible
    return dict(nativeOutcome=outcome, executionFailed=failed,
        normalizationRejected=any(document.get("normalizationStatus") == "rejected"
            for document in (observation.initial, observation.reviewed)),
        measurementComplete=peak is not None,
        cost=dict(task=task_id, tool=tool_id, arm=arm_id,
            firstPageBytes=len(observation.stdout), workflowBytes=len(observation.stdout),
            workflowWallMs=elapsed, followUps=0, overBudget=overflow, failed=failed,
            peakRssBytes=peak, condition="fresh_cli"))
