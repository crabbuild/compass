#!/usr/bin/env python3
"""Bounded Python framework source oracle used only by qualification.

The oracle parses source with the Python standard library. It never imports a
corpus module, executes repository code, installs dependencies, or contacts a
service. Every scanned file receives an explicit status and every construct
keeps its exact UTF-8 byte range.
"""

from __future__ import annotations

import argparse
import ast
import hashlib
import io
import json
import sys
import tokenize
from collections import Counter
from pathlib import Path
from typing import Any, Iterable


SCHEMA = "compass.python-framework-source-oracle/1"
MAX_FILES = 20_000
MAX_FILE_BYTES = 8 * 1024 * 1024
MAX_RECORDS = 2_000_000
SKIP_DIRECTORIES = frozenset(
    {".git", ".hg", ".svn", ".tox", ".venv", "venv", "node_modules", "__pycache__"}
)
ROUTE_METHODS = frozenset(
    {"route", "api_route", "get", "post", "put", "patch", "delete", "options", "head", "websocket"}
)
MOUNT_CALLS = frozenset({"include", "include_router", "register_blueprint", "mount", "Mount"})
DEPENDENCY_CALLS = frozenset({"Depends", "Security"})
ORM_CALLS = frozenset(
    {"mapped_column", "relationship", "ForeignKey", "Column", "Table", "ManyToManyField", "ForeignKey"}
)
JOB_CALLS = frozenset(
    {"delay", "apply_async", "send_task", "signature", "chain", "group", "chord", "retry"}
)
MODEL_BASES = frozenset(
    {
        "BaseModel",
        "Model",
        "DeclarativeBase",
        "Serializer",
        "ModelSerializer",
        "ViewSet",
        "ModelViewSet",
        "APIView",
        "MethodView",
    }
)


class OracleError(RuntimeError):
    """A bounded qualification-oracle failure."""


def canonical_bytes(value: Any) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True) + "\n").encode()


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def source_files(root: Path) -> list[Path]:
    files = sorted(
        path
        for path in root.rglob("*")
        if path.is_file()
        and path.suffix in {".py", ".pyi"}
        and not SKIP_DIRECTORIES.intersection(path.relative_to(root).parts)
    )
    if len(files) > MAX_FILES:
        raise OracleError(f"Python source file count {len(files)} exceeds {MAX_FILES}")
    return files


def dotted_name(node: ast.AST | None) -> str | None:
    if isinstance(node, ast.Name):
        return node.id
    if isinstance(node, ast.Attribute):
        owner = dotted_name(node.value)
        return f"{owner}.{node.attr}" if owner else node.attr
    if isinstance(node, ast.Call):
        called = dotted_name(node.func)
        return f"{called}()" if called else None
    return None


class InventoryVisitor(ast.NodeVisitor):
    def __init__(self, source: bytes, line_starts: list[int]) -> None:
        self.source = source
        self.line_starts = line_starts
        self.records: list[dict[str, Any]] = []
        self.owners: list[str] = []

    def anchor(self, node: ast.AST) -> dict[str, int]:
        start_line = getattr(node, "lineno", 1)
        end_line = getattr(node, "end_lineno", start_line)
        start_column = getattr(node, "col_offset", 0)
        end_column = getattr(node, "end_col_offset", start_column)
        start = self.line_starts[start_line - 1] + start_column
        end = self.line_starts[end_line - 1] + end_column
        if not 0 <= start < end <= len(self.source):
            raise OracleError(f"invalid AST range {start}:{end} for {type(node).__name__}")
        return {
            "startByte": start,
            "endByte": end,
            "startLine": start_line,
            "startColumn": start_column,
            "endLine": end_line,
            "endColumn": end_column,
        }

    def add(self, kind: str, node: ast.AST, **detail: Any) -> None:
        if len(self.records) >= MAX_RECORDS:
            raise OracleError(f"Python construct count exceeds {MAX_RECORDS}")
        record: dict[str, Any] = {
            "kind": kind,
            "anchor": self.anchor(node),
        }
        if self.owners:
            record["owner"] = ".".join(self.owners)
        record.update({key: value for key, value in detail.items() if value is not None})
        self.records.append(record)

    def visit_ClassDef(self, node: ast.ClassDef) -> None:  # noqa: N802
        bases = [name for base in node.bases if (name := dotted_name(base))]
        self.add("declaration", node, declarationKind="class", name=node.name)
        for base in node.bases:
            self.add("base", base, target=dotted_name(base))
        if any(base.rsplit(".", 1)[-1] in MODEL_BASES for base in bases):
            self.add("framework_role", node, role="model", name=node.name, bases=bases)
        self.owners.append(node.name)
        self.generic_visit(node)
        self.owners.pop()

    def _visit_function(self, node: ast.FunctionDef | ast.AsyncFunctionDef) -> None:
        kind = "async_function" if isinstance(node, ast.AsyncFunctionDef) else "function"
        self.add("declaration", node, declarationKind=kind, name=node.name)
        arguments: Iterable[ast.arg] = (
            list(node.args.posonlyargs)
            + list(node.args.args)
            + list(node.args.kwonlyargs)
            + ([node.args.vararg] if node.args.vararg else [])
            + ([node.args.kwarg] if node.args.kwarg else [])
        )
        for argument in arguments:
            self.add("parameter", argument, name=argument.arg, annotation=dotted_name(argument.annotation))
        if node.returns is not None:
            self.add("annotation", node.returns, context="return", target=dotted_name(node.returns))
        for decorator in node.decorator_list:
            self.add("decorator", decorator, target=dotted_name(decorator))
            called = decorator.func if isinstance(decorator, ast.Call) else decorator
            terminal = (dotted_name(called) or "").rsplit(".", 1)[-1]
            if terminal in ROUTE_METHODS:
                self.add("route_registration", decorator, registration=dotted_name(called), handler=node.name)
            if terminal in {"task", "shared_task"}:
                self.add("task_registration", decorator, registration=dotted_name(called), handler=node.name)
        self.owners.append(node.name)
        self.generic_visit(node)
        self.owners.pop()

    def visit_FunctionDef(self, node: ast.FunctionDef) -> None:  # noqa: N802
        self._visit_function(node)

    def visit_AsyncFunctionDef(self, node: ast.AsyncFunctionDef) -> None:  # noqa: N802
        self._visit_function(node)

    def visit_Import(self, node: ast.Import) -> None:  # noqa: N802
        for alias in node.names:
            self.add("import", node, module=alias.name, alias=alias.asname)

    def visit_ImportFrom(self, node: ast.ImportFrom) -> None:  # noqa: N802
        for alias in node.names:
            self.add(
                "import",
                node,
                module=("." * node.level) + (node.module or ""),
                name=alias.name,
                alias=alias.asname,
            )

    def visit_AnnAssign(self, node: ast.AnnAssign) -> None:  # noqa: N802
        self.add("assignment", node, target=dotted_name(node.target), annotation=dotted_name(node.annotation))
        self.add("annotation", node.annotation, context="assignment", target=dotted_name(node.annotation))
        self.generic_visit(node)

    def visit_Assign(self, node: ast.Assign) -> None:  # noqa: N802
        for target in node.targets:
            self.add("assignment", target, target=dotted_name(target))
        self.generic_visit(node)

    def visit_Return(self, node: ast.Return) -> None:  # noqa: N802
        self.add("return", node, value=dotted_name(node.value))
        self.generic_visit(node)

    def visit_Attribute(self, node: ast.Attribute) -> None:  # noqa: N802
        self.add("member_access", node, member=node.attr, receiver=dotted_name(node.value))
        self.generic_visit(node)

    def visit_Call(self, node: ast.Call) -> None:  # noqa: N802
        called = dotted_name(node.func)
        terminal = (called or "").rsplit(".", 1)[-1]
        self.add("call", node, target=called, positional=len(node.args), keywords=len(node.keywords))
        if terminal in ROUTE_METHODS or terminal in {"path", "re_path", "url", "add_url_rule", "Route", "WebSocketRoute"}:
            self.add("route_registration", node, registration=called)
        if terminal in MOUNT_CALLS:
            self.add("mount_registration", node, registration=called)
        if terminal in DEPENDENCY_CALLS:
            self.add("dependency_provider", node, provider=dotted_name(node.args[0]) if node.args else None)
        if terminal in ORM_CALLS:
            self.add("orm_mapping", node, registration=called)
        if terminal in JOB_CALLS:
            self.add("job_topology", node, operation=terminal, target=called)
        self.generic_visit(node)


def line_starts(source: bytes) -> list[int]:
    starts = [0]
    starts.extend(index + 1 for index, byte in enumerate(source) if byte == 0x0A)
    return starts


def token_count(source: bytes) -> int:
    try:
        return sum(1 for _token in tokenize.tokenize(io.BytesIO(source).readline))
    except (IndentationError, SyntaxError, tokenize.TokenError):
        return 0


def inventory_file(path: Path, root: Path) -> dict[str, Any]:
    relative = path.relative_to(root).as_posix()
    source = path.read_bytes()
    if len(source) > MAX_FILE_BYTES:
        return {
            "path": relative,
            "status": "limit_exceeded",
            "bytes": len(source),
            "sourceSha256": sha256(source),
            "constructs": [],
            "reason": f"file exceeds {MAX_FILE_BYTES} bytes",
        }
    try:
        text = source.decode("utf-8")
    except UnicodeDecodeError as error:
        return {
            "path": relative,
            "status": "partial",
            "bytes": len(source),
            "sourceSha256": sha256(source),
            "constructs": [],
            "reason": f"invalid UTF-8 at byte {error.start}",
        }
    try:
        tree = ast.parse(text, filename=relative, type_comments=True)
    except (SyntaxError, ValueError) as error:
        return {
            "path": relative,
            "status": "partial",
            "bytes": len(source),
            "sourceSha256": sha256(source),
            "tokenCount": token_count(source),
            "constructs": [],
            "reason": f"{type(error).__name__}: {error}",
        }
    visitor = InventoryVisitor(source, line_starts(source))
    visitor.visit(tree)
    visitor.records.sort(
        key=lambda record: (
            record["anchor"]["startByte"],
            record["anchor"]["endByte"],
            record["kind"],
            json.dumps(record, sort_keys=True, separators=(",", ":")),
        )
    )
    return {
        "path": relative,
        "status": "ok",
        "bytes": len(source),
        "sourceSha256": sha256(source),
        "tokenCount": token_count(source),
        "constructs": visitor.records,
    }


def build_inventory(root: Path) -> dict[str, Any]:
    root = root.resolve()
    if not root.is_dir():
        raise OracleError(f"Python corpus root does not exist: {root}")
    files = [inventory_file(path, root) for path in source_files(root)]
    kinds = Counter(
        record["kind"]
        for file_record in files
        for record in file_record["constructs"]
    )
    document: dict[str, Any] = {
        "schema": SCHEMA,
        "parser": {
            "id": "python-stdlib-ast-tokenize",
            "version": f"{sys.version_info.major}.{sys.version_info.minor}.{sys.version_info.micro}",
        },
        "limits": {
            "maxFiles": MAX_FILES,
            "maxFileBytes": MAX_FILE_BYTES,
            "maxRecords": MAX_RECORDS,
        },
        "summary": {
            "files": len(files),
            "partialFiles": sum(file_record["status"] != "ok" for file_record in files),
            "constructs": sum(kinds.values()),
            "constructKinds": dict(sorted(kinds.items())),
        },
        "files": files,
    }
    document["inventorySha256"] = sha256(canonical_bytes(document).rstrip(b"\n"))
    return document


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args(argv)
    try:
        inventory = build_inventory(arguments.root)
        arguments.output.parent.mkdir(parents=True, exist_ok=True)
        arguments.output.write_bytes(canonical_bytes(inventory))
    except (OSError, OracleError) as error:
        print(f"python framework oracle: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
