"""Public command contracts for Compass and optional Graphify qualification."""

from __future__ import annotations

from dataclasses import dataclass
import hashlib
import os
from pathlib import Path
import re
import subprocess
import sys
import venv

from .model import RepositorySpec, ToolRevision
from .workspace import (
    QualificationWorkspace,
    guarded_remove,
    prepare_checkout,
    resolve_remote_head,
)

_TIMING = re.compile(r"^\[compass timing\] ([^:]+): ([0-9]+(?:\.[0-9]+)?)s$")


def _run(arguments: list[str], *, cwd: Path) -> str:
    completed = subprocess.run(
        arguments,
        cwd=cwd,
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8",
    )
    if completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip()
        raise RuntimeError(f"{' '.join(arguments)} failed: {detail}")
    return completed.stdout.strip()


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _git_value(root: Path, *arguments: str) -> str:
    return _run(["git", *arguments], cwd=root)


def _revision(name: str, root: Path, binary: Path) -> ToolRevision:
    status = _git_value(root, "status", "--porcelain=v1", "--untracked-files=all")
    if status:
        raise RuntimeError(f"{name} source checkout must be clean:\n{status}")
    return ToolRevision(
        name=name,
        url=_git_value(root, "remote", "get-url", "origin"),
        commit=_git_value(root, "rev-parse", "HEAD"),
        tree=_git_value(root, "rev-parse", "HEAD^{tree}"),
        dirty=False,
        binary_sha256=_sha256(binary),
        metadata={},
    )


@dataclass(frozen=True)
class ToolAdapter:
    executable: Path
    revision: ToolRevision

    @property
    def name(self) -> str:
        return self.revision.name

    def build_command(self, checkout: Path, output: Path, *, force: bool = False) -> tuple[str, ...]:
        raise NotImplementedError

    def query_command(self, graph: Path, question: str) -> tuple[str, ...]:
        raise NotImplementedError

    def compassql_command(self, graph: Path, query: str) -> tuple[str, ...]:
        raise RuntimeError(f"{self.name} does not support CompassQL qualification")

    def graph_path(self, output: Path) -> Path:
        raise NotImplementedError

    def parse_build_evidence(self, stderr: str) -> dict[str, float]:
        return {}

    def cleanup_checkout(self, checkout: Path) -> None:
        """Remove tool-owned checkout side effects after a measured command."""

    def prune_superseded_artifacts(self, output: Path, active_graph: Path) -> None:
        """Release tool-specific artifacts that are no longer needed by the run."""


@dataclass(frozen=True)
class CompassAdapter(ToolAdapter):
    @classmethod
    def prepare(cls, source_root: Path) -> "CompassAdapter":
        status = _git_value(source_root, "status", "--porcelain=v1", "--untracked-files=all")
        if status:
            raise RuntimeError(f"Compass source checkout must be clean:\n{status}")
        _run(
            [
                "cargo",
                "build",
                "--release",
                "--locked",
                "-p",
                "compass-cli",
                "--bin",
                "compass",
            ],
            cwd=source_root,
        )
        binary = source_root / "target" / "release" / "compass"
        if not binary.is_file() or not os.access(binary, os.X_OK):
            raise RuntimeError(f"release Compass binary is not executable: {binary}")
        revision = _revision("compass", source_root, binary)
        metadata = dict(revision.metadata)
        metadata["rustc"] = _run(["rustc", "--version"], cwd=source_root)
        metadata["cargo"] = _run(["cargo", "--version"], cwd=source_root)
        metadata["profile"] = "release"
        return cls(binary, ToolRevision(**{**revision.__dict__, "metadata": metadata}))

    def build_command(self, checkout: Path, output: Path, *, force: bool = False) -> tuple[str, ...]:
        command = [
            str(self.executable),
            "extract",
            str(checkout),
            "--code-only",
            "--timing",
            "--out",
            str(output),
        ]
        if force:
            command.append("--force")
        return tuple(command)

    def query_command(self, graph: Path, question: str) -> tuple[str, ...]:
        return (str(self.executable), "query", question, "--graph", str(graph))

    def compassql_command(self, graph: Path, query: str) -> tuple[str, ...]:
        return (
            str(self.executable),
            "query",
            "--cql",
            query,
            "--graph",
            str(graph),
            "--format",
            "json",
        )

    def graph_path(self, output: Path) -> Path:
        compass_output = output / "compass-out"
        pointer = compass_output / ".compass-active-generation"
        try:
            generation = pointer.read_text(encoding="utf-8").strip()
        except OSError as error:
            raise RuntimeError(f"missing Compass generation pointer: {pointer}") from error
        if not generation.startswith("generation-") or Path(generation).name != generation:
            raise RuntimeError(f"invalid Compass generation pointer: {generation!r}")
        active = compass_output / ".compass-generations" / generation
        if not active.is_dir() or (active / ".compass-build-incomplete").exists():
            raise RuntimeError(f"incomplete Compass generation: {active}")
        graph = active / "graph.json"
        if not graph.is_file() or not graph.resolve().is_relative_to(output.resolve()):
            raise RuntimeError(f"invalid Compass graph artifact: {graph}")
        return graph

    def parse_build_evidence(self, stderr: str) -> dict[str, float]:
        evidence: dict[str, float] = {}
        for line in stderr.splitlines():
            matched = _TIMING.fullmatch(line.strip())
            if matched:
                evidence[matched.group(1).replace(" ", "_")] = float(matched.group(2))
        return evidence

    def prune_superseded_artifacts(self, output: Path, active_graph: Path) -> None:
        generations = output / "compass-out" / ".compass-generations"
        active = active_graph.parent.resolve()
        if not generations.is_dir() or active.parent.resolve() != generations.resolve():
            raise RuntimeError(f"active Compass generation is outside {generations}")
        for candidate in generations.iterdir():
            if candidate.is_dir() and candidate.resolve() != active:
                guarded_remove(candidate)


@dataclass(frozen=True)
class GraphifyAdapter(ToolAdapter):
    @classmethod
    def prepare(
        cls,
        workspace: QualificationWorkspace,
        url: str = "https://github.com/Graphify-Labs/graphify.git",
    ) -> "GraphifyAdapter":
        branch, commit = resolve_remote_head(url)
        spec = RepositorySpec("graphify", url, ".py", ())
        checkout = workspace.root / "tools" / "graphify-source"
        identity = prepare_checkout(spec, commit, checkout)
        if identity.branch != branch:
            raise RuntimeError("Graphify default branch changed during preparation")
        environment = workspace.root / "tools" / "graphify-venv"
        if environment.exists():
            guarded_remove(environment)
        venv.EnvBuilder(with_pip=True, clear=False).create(environment)
        python = environment / ("Scripts/python.exe" if os.name == "nt" else "bin/python")
        _run(
            [
                str(python),
                "-m",
                "pip",
                "install",
                "--disable-pip-version-check",
                str(checkout),
            ],
            cwd=workspace.root,
        )
        revision = _revision("graphify", checkout, python)
        metadata = dict(revision.metadata)
        metadata["python"] = _run([str(python), "--version"], cwd=workspace.root)
        return cls(python, ToolRevision(**{**revision.__dict__, "metadata": metadata}))

    def build_command(self, checkout: Path, output: Path, *, force: bool = False) -> tuple[str, ...]:
        command = [
            str(self.executable),
            "-m",
            "graphify",
            "extract",
            str(checkout),
            "--code-only",
            "--out",
            str(output),
        ]
        if force:
            command.append("--force")
        return tuple(command)

    def query_command(self, graph: Path, question: str) -> tuple[str, ...]:
        return (
            str(self.executable),
            "-m",
            "graphify",
            "query",
            question,
            "--graph",
            str(graph),
        )

    def graph_path(self, output: Path) -> Path:
        graph = output / "graphify-out" / "graph.json"
        if not graph.is_file() or not graph.resolve().is_relative_to(output.resolve()):
            raise RuntimeError(f"invalid Graphify graph artifact: {graph}")
        return graph

    def cleanup_checkout(self, checkout: Path) -> None:
        generated = checkout / "graphify-out"
        if generated.exists() or generated.is_symlink():
            guarded_remove(generated)
