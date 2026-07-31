"""Public per-command measurement API."""

from __future__ import annotations

from dataclasses import dataclass
import json
import os
from pathlib import Path
import subprocess
import sys

from .model import ProcessMetrics


@dataclass(frozen=True)
class ProcessSpec:
    command: tuple[str, ...]
    cwd: Path
    stdout_path: Path
    stderr_path: Path
    timeout_seconds: float
    env: dict[str, str] | None = None


def run_measured(spec: ProcessSpec) -> ProcessMetrics:
    if not spec.command:
        raise ValueError("measurement command cannot be empty")
    if spec.timeout_seconds <= 0:
        raise ValueError("measurement timeout must be positive")
    worker = Path(__file__).with_name("measure_child.py")
    command = [
        sys.executable,
        str(worker),
        "--cwd",
        str(spec.cwd),
        "--stdout",
        str(spec.stdout_path),
        "--stderr",
        str(spec.stderr_path),
        "--timeout-seconds",
        str(spec.timeout_seconds),
        "--",
        *spec.command,
    ]
    environment = os.environ.copy()
    if spec.env is not None:
        environment.update(spec.env)
    completed = subprocess.run(
        command,
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8",
        env=environment,
        timeout=spec.timeout_seconds + 15,
    )
    if completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip()
        raise RuntimeError(f"measurement worker failed: {detail}")
    try:
        value = json.loads(completed.stdout)
        return ProcessMetrics(
            wall_seconds=float(value["wall_seconds"]),
            user_seconds=float(value["user_seconds"]),
            system_seconds=float(value["system_seconds"]),
            peak_rss_kib=int(value["peak_rss_kib"]),
            return_code=int(value["return_code"]),
            signal=None if value["signal"] is None else int(value["signal"]),
            timed_out=bool(value["timed_out"]),
            command=tuple(str(item) for item in value["command"]),
            cwd=str(value["cwd"]),
            stdout_path=str(value["stdout_path"]),
            stderr_path=str(value["stderr_path"]),
            stdout_sha256=str(value["stdout_sha256"]),
            stderr_sha256=str(value["stderr_sha256"]),
        )
    except (KeyError, TypeError, ValueError, json.JSONDecodeError) as error:
        raise RuntimeError(f"invalid measurement worker output: {completed.stdout!r}") from error
