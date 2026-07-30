"""Typed configuration and result records for performance qualification."""

from __future__ import annotations

from dataclasses import asdict, dataclass, field
from typing import Any


@dataclass(frozen=True)
class QueryOracle:
    question: str
    required: tuple[str, ...]
    forbidden: tuple[str, ...] = ()


@dataclass(frozen=True)
class RepositorySpec:
    name: str
    url: str
    mutation_suffix: str
    queries: tuple[QueryOracle, ...]


@dataclass(frozen=True)
class Suite:
    schema: str
    repositories: tuple[RepositorySpec, ...]
    digest: str


@dataclass(frozen=True)
class CheckoutIdentity:
    name: str
    url: str
    branch: str
    commit: str
    tree: str
    path: str


@dataclass(frozen=True)
class ToolRevision:
    name: str
    url: str
    commit: str
    tree: str
    dirty: bool
    binary_sha256: str
    metadata: dict[str, str] = field(default_factory=dict)


@dataclass(frozen=True)
class EnvironmentIdentity:
    system: str
    release: str
    architecture: str
    cpu_model: str
    physical_cores: int
    logical_cores: int
    total_memory_bytes: int
    python_version: str
    rust_version: str
    cargo_version: str
    hostname: str
    runner_id: str


@dataclass(frozen=True)
class ProcessMetrics:
    wall_seconds: float
    user_seconds: float
    system_seconds: float
    peak_rss_kib: int
    return_code: int
    signal: int | None
    timed_out: bool
    command: tuple[str, ...]
    cwd: str
    stdout_path: str
    stderr_path: str
    stdout_sha256: str
    stderr_sha256: str


@dataclass(frozen=True)
class Sample:
    sample_id: str
    tool: str
    repository: str
    workload: str
    iteration: int
    eligible: bool
    metrics: ProcessMetrics
    correctness_digest: str = ""
    error: str | None = None
    evidence: dict[str, float | int | str | bool] = field(default_factory=dict)


@dataclass(frozen=True)
class Aggregate:
    samples: int
    p50_seconds: float
    p95_seconds: float
    min_seconds: float
    max_seconds: float
    mad_seconds: float
    peak_rss_kib: int


@dataclass(frozen=True)
class CorrectnessResult:
    passed: bool
    digest: str
    failures: tuple[str, ...] = ()
    warnings: tuple[str, ...] = ()
    metrics: dict[str, int | str | bool] = field(default_factory=dict)


@dataclass(frozen=True)
class WorkloadResult:
    tool: str
    repository: str
    workload: str
    samples: tuple[Sample, ...]
    aggregate: Aggregate | None
    correctness: CorrectnessResult


@dataclass(frozen=True)
class GateIssue:
    code: str
    repository: str
    workload: str
    message: str


@dataclass(frozen=True)
class GateReport:
    passed: bool
    issues: tuple[GateIssue, ...]
    ratios: dict[str, float] = field(default_factory=dict)


@dataclass(frozen=True)
class QualificationRun:
    schema: str
    run_id: str
    started_at: str
    completed_at: str | None
    complete: bool
    suite_digest: str
    environment: EnvironmentIdentity
    tools: tuple[ToolRevision, ...]
    corpora: tuple[CheckoutIdentity, ...]
    results: tuple[WorkloadResult, ...]
    gates: GateReport | None = None


def to_json_value(value: Any) -> Any:
    """Convert nested qualification dataclasses to JSON-safe values."""
    if hasattr(value, "__dataclass_fields__"):
        return {key: to_json_value(item) for key, item in asdict(value).items()}
    if isinstance(value, tuple):
        return [to_json_value(item) for item in value]
    if isinstance(value, dict):
        return {str(key): to_json_value(item) for key, item in value.items()}
    return value


def run_from_json_value(value: dict[str, Any]) -> QualificationRun:
    """Rehydrate a persisted qualification run for reporting and comparison."""

    def process(item: dict[str, Any]) -> ProcessMetrics:
        return ProcessMetrics(
            **{
                **item,
                "command": tuple(item["command"]),
            }
        )

    def sample(item: dict[str, Any]) -> Sample:
        return Sample(**{**item, "metrics": process(item["metrics"])})

    def correctness(item: dict[str, Any]) -> CorrectnessResult:
        return CorrectnessResult(
            **{
                **item,
                "failures": tuple(item.get("failures", ())),
                "warnings": tuple(item.get("warnings", ())),
            }
        )

    def result(item: dict[str, Any]) -> WorkloadResult:
        aggregate = item.get("aggregate")
        return WorkloadResult(
            tool=item["tool"],
            repository=item["repository"],
            workload=item["workload"],
            samples=tuple(sample(entry) for entry in item["samples"]),
            aggregate=Aggregate(**aggregate) if aggregate is not None else None,
            correctness=correctness(item["correctness"]),
        )

    gates = value.get("gates")
    gate_report = None
    if gates is not None:
        gate_report = GateReport(
            passed=gates["passed"],
            issues=tuple(GateIssue(**issue) for issue in gates.get("issues", ())),
            ratios=dict(gates.get("ratios", {})),
        )
    return QualificationRun(
        schema=value["schema"],
        run_id=value["run_id"],
        started_at=value["started_at"],
        completed_at=value.get("completed_at"),
        complete=value["complete"],
        suite_digest=value["suite_digest"],
        environment=EnvironmentIdentity(**value["environment"]),
        tools=tuple(ToolRevision(**tool) for tool in value["tools"]),
        corpora=tuple(CheckoutIdentity(**corpus) for corpus in value["corpora"]),
        results=tuple(result(item) for item in value["results"]),
        gates=gate_report,
    )
