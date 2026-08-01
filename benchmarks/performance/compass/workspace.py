"""Owned qualification workspaces, locks, and exact Git checkouts."""

from __future__ import annotations

from contextlib import contextmanager
from dataclasses import asdict
import json
import os
from pathlib import Path
import re
import shutil
import socket
import subprocess
import time
from typing import Iterator

from . import WORKSPACE_SCHEMA
from .model import CheckoutIdentity, RepositorySpec

_OBJECT_ID = re.compile(r"^[0-9a-f]{40}$")
_MARKER = ".compass-performance-workspace.json"


def _git(arguments: list[str], *, cwd: Path | None = None) -> str:
    completed = subprocess.run(
        ["git", *arguments],
        cwd=cwd,
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8",
    )
    if completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip()
        raise RuntimeError(f"git {' '.join(arguments)} failed: {detail}")
    return completed.stdout.strip()


def _write_json_atomic(path: Path, value: object) -> None:
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    with temporary.open("w", encoding="utf-8") as stream:
        json.dump(value, stream, sort_keys=True, separators=(",", ":"))
        stream.write("\n")
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(temporary, path)


class QualificationWorkspace:
    def __init__(self, root: Path):
        self.root = root.resolve()
        self.marker = self.root / _MARKER
        self.lock = self.root / ".qualification.lock"

    @classmethod
    def create(cls, path: Path) -> "QualificationWorkspace":
        root = path.resolve()
        forbidden = {Path("/").resolve(), Path.home().resolve()}
        if root in forbidden or root.name in {".git", "compass"}:
            raise ValueError(f"unsafe qualification workspace: {root}")
        root.mkdir(parents=True, exist_ok=True)
        workspace = cls(root)
        if workspace.marker.exists():
            workspace.validate()
        else:
            _write_json_atomic(
                workspace.marker,
                {"schema": WORKSPACE_SCHEMA, "root": str(workspace.root)},
            )
        return workspace

    def validate(self) -> None:
        try:
            value = json.loads(self.marker.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            raise ValueError(f"invalid workspace marker: {self.marker}") from error
        if value != {"schema": WORKSPACE_SCHEMA, "root": str(self.root)}:
            raise ValueError(f"workspace marker does not own {self.root}")

    @contextmanager
    def acquire(self) -> Iterator[None]:
        self.validate()
        payload = {
            "pid": os.getpid(),
            "hostname": socket.gethostname(),
            "started_at_unix": time.time(),
        }
        try:
            descriptor = os.open(
                self.lock,
                os.O_CREAT | os.O_EXCL | os.O_WRONLY,
                0o600,
            )
        except FileExistsError as error:
            owner = self.lock.read_text(encoding="utf-8", errors="replace")
            raise RuntimeError(f"qualification workspace is locked: {owner.strip()}") from error
        try:
            with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
                json.dump(payload, stream, sort_keys=True)
                stream.write("\n")
                stream.flush()
                os.fsync(stream.fileno())
            yield
        finally:
            try:
                self.lock.unlink()
            except FileNotFoundError:
                pass


def _workspace_for(path: Path) -> QualificationWorkspace:
    candidate = Path(os.path.abspath(path))
    for parent in (candidate.parent, *candidate.parents):
        marker = parent / _MARKER
        if marker.is_file():
            workspace = QualificationWorkspace(parent)
            workspace.validate()
            return workspace
    raise ValueError(f"no qualification workspace owns {path}")


def guarded_remove(path: Path) -> None:
    workspace = _workspace_for(path)
    resolved = path.resolve(strict=False)
    try:
        relative = resolved.relative_to(workspace.root)
    except ValueError as error:
        raise ValueError(f"destructive target escapes workspace: {resolved}") from error
    if len(relative.parts) < 2 or ".git" in relative.parts:
        raise ValueError(f"destructive target is too broad: {resolved}")
    if not path.exists() and not path.is_symlink():
        return
    if path.is_symlink() or path.is_file():
        path.unlink()
    elif path.is_dir():
        shutil.rmtree(path)
    else:
        raise ValueError(f"unsupported destructive target: {path}")


def resolve_remote_head(url: str) -> tuple[str, str]:
    output = _git(["ls-remote", "--symref", url, "HEAD"])
    branch: str | None = None
    commit: str | None = None
    for line in output.splitlines():
        if line.startswith("ref: refs/heads/") and line.endswith("\tHEAD"):
            branch = line.removeprefix("ref: refs/heads/").removesuffix("\tHEAD")
        elif line.endswith("\tHEAD"):
            candidate = line.split("\t", 1)[0]
            if _OBJECT_ID.fullmatch(candidate):
                commit = candidate
    if branch is None or commit is None:
        raise RuntimeError(f"remote HEAD is not a symbolic branch: {url}")
    return branch, commit


def prepare_checkout(
    spec: RepositorySpec,
    commit: str,
    destination: Path,
    *,
    pinned: bool = False,
) -> CheckoutIdentity:
    if _OBJECT_ID.fullmatch(commit) is None:
        raise ValueError(f"invalid commit for {spec.name}: {commit}")
    workspace = _workspace_for(destination)
    if destination.exists() or destination.is_symlink():
        guarded_remove(destination)
    destination.parent.mkdir(parents=True, exist_ok=True)
    _git(["clone", "--quiet", "--no-checkout", spec.url, str(destination)])
    _git(["checkout", "--quiet", "--detach", commit], cwd=destination)
    status = _git(["status", "--porcelain=v1", "--untracked-files=all"], cwd=destination)
    if status:
        raise RuntimeError(f"prepared checkout is dirty: {destination}")
    actual = _git(["rev-parse", "HEAD"], cwd=destination)
    if actual != commit:
        raise RuntimeError(f"checkout resolved {actual}, expected {commit}")
    branch, remote_commit = resolve_remote_head(spec.url)
    if not pinned and remote_commit != commit:
        raise RuntimeError(
            f"remote HEAD changed while preparing {spec.name}: {commit} -> {remote_commit}"
        )
    tree = _git(["rev-parse", f"{commit}^{{tree}}"], cwd=destination)
    identity = CheckoutIdentity(
        name=spec.name,
        url=spec.url,
        branch=branch,
        commit=commit,
        tree=tree,
        path=str(destination.resolve()),
    )
    identity_path = workspace.root / "identities" / f"{spec.name}.json"
    identity_path.parent.mkdir(parents=True, exist_ok=True)
    _write_json_atomic(identity_path, asdict(identity))
    return identity
