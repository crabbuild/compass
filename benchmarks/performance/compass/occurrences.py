"""Repository-rooted source statement evidence for graph comparison."""

from __future__ import annotations

import ast
from collections.abc import Callable, Mapping
from dataclasses import dataclass
import fnmatch
import hashlib
import json
import os
from pathlib import Path
import re
import selectors
import subprocess
import sys
import tempfile
import time
import tokenize

from .jsonstream import iter_top_level_array, read_top_level_value


StatementSpans = Mapping[str, tuple[tuple[int, int], ...]]
StatementProvider = Callable[[Path], StatementSpans]
_LOCATION = re.compile(r"L([1-9][0-9]*)\Z")


@dataclass(frozen=True)
class SourceConstruct:
    """One independently parsed source occurrence awaiting adjudication."""

    source_file: str
    relation: str
    capability: str
    owner_qualified_name: str
    target_spelling: str
    qualifier: str | None
    start_byte: int
    end_byte: int
    start_line: int
    framework_pack: str | None = None


def _source_construct_key(construct: SourceConstruct) -> tuple[object, ...]:
    return (
        construct.source_file,
        construct.relation,
        construct.capability,
        construct.owner_qualified_name,
        construct.target_spelling,
        construct.qualifier or "",
        construct.start_byte,
        construct.end_byte,
        construct.start_line,
        construct.framework_pack or "",
    )


@dataclass(frozen=True)
class SourceConstructInventory:
    constructs: tuple[SourceConstruct, ...]
    scanned_files: int
    parsed_files: int
    rejected_files: tuple[str, ...]
    provider_metadata: tuple[tuple[str, str], ...] = ()


SourceConstructParser = Callable[
    [Path, Path],
    tuple[SourceConstruct, ...] | None,
]


@dataclass(frozen=True)
class ConstructProvider:
    identity: str
    suffixes: tuple[str, ...]
    parse: SourceConstructParser
    collect: Callable[[Path, tuple[str, ...], tuple[str, ...]], SourceConstructInventory] | None = None


def _python_statement_spans(path: Path) -> StatementSpans:
    try:
        with tokenize.open(path) as source:
            tree = ast.parse(source.read(), filename=str(path))
    except (OSError, SyntaxError, UnicodeError):
        return {}

    imports: list[tuple[int, int]] = []
    calls: list[tuple[int, int]] = []
    for node in ast.walk(tree):
        end = getattr(node, "end_lineno", None)
        if not isinstance(end, int) or end < node.lineno:
            continue
        span = (node.lineno, end)
        if isinstance(node, (ast.Import, ast.ImportFrom)):
            imports.append(span)
        elif isinstance(node, ast.Call):
            calls.append(span)
    return {
        "imports": tuple(sorted(set(imports))),
        "calls": tuple(sorted(set(calls))),
    }


def _python_module(root: Path, path: Path) -> str:
    relative = path.relative_to(root).with_suffix("")
    parts = list(relative.parts)
    if parts and parts[-1] == "__init__":
        parts.pop()
    return ".".join(parts)


def _python_name(node: ast.AST, source: str) -> tuple[str, str | None]:
    if isinstance(node, ast.Name):
        return node.id, None
    if isinstance(node, ast.Attribute):
        qualifier = ast.get_source_segment(source, node.value) or ""
        return node.attr, qualifier or None
    spelling = ast.get_source_segment(source, node) or ""
    return spelling.strip(), None


def _python_constructs(root: Path, path: Path) -> tuple[SourceConstruct, ...] | None:
    try:
        raw = path.read_bytes()
        bom = len(tokenize.BOM_UTF8) if raw.startswith(tokenize.BOM_UTF8) else 0
        source = raw.decode("utf-8-sig")
        tree = ast.parse(source, filename=str(path))
    except (OSError, SyntaxError, UnicodeError):
        return None

    encoded_lines = [line.encode("utf-8") for line in source.splitlines(keepends=True)]
    line_offsets: list[int] = []
    offset = bom
    for line in encoded_lines:
        line_offsets.append(offset)
        offset += len(line)

    def byte_range(node: ast.AST) -> tuple[int, int, int] | None:
        start_line = getattr(node, "lineno", None)
        end_line = getattr(node, "end_lineno", None)
        start_column = getattr(node, "col_offset", None)
        end_column = getattr(node, "end_col_offset", None)
        if (
            not isinstance(start_line, int)
            or not isinstance(end_line, int)
            or not isinstance(start_column, int)
            or not isinstance(end_column, int)
            or start_line < 1
            or end_line < start_line
            or end_line > len(line_offsets)
        ):
            return None
        start = line_offsets[start_line - 1] + start_column
        end = line_offsets[end_line - 1] + end_column
        if start < bom or end <= start or end > len(raw):
            return None
        return start, end, start_line

    module = _python_module(root, path)
    relative = path.relative_to(root).as_posix()
    constructs: list[SourceConstruct] = []

    class Visitor(ast.NodeVisitor):
        def __init__(self) -> None:
            self.owners = [module]

        @property
        def owner(self) -> str:
            return ".".join(part for part in self.owners if part)

        def _nested(self, name: str, body: list[ast.stmt]) -> None:
            self.owners.append(name)
            for statement in body:
                self.visit(statement)
            self.owners.pop()

        def visit_ClassDef(self, node: ast.ClassDef) -> None:
            for decorator in node.decorator_list:
                self.visit(decorator)
            for base in node.bases:
                self.visit(base)
            for keyword in node.keywords:
                self.visit(keyword.value)
            for type_parameter in getattr(node, "type_params", ()):  # Python 3.12+
                self.visit(type_parameter)
            self._nested(node.name, node.body)

        def visit_FunctionDef(self, node: ast.FunctionDef) -> None:
            self._visit_function_definition_expressions(node)
            self._nested(node.name, node.body)

        def visit_AsyncFunctionDef(self, node: ast.AsyncFunctionDef) -> None:
            self._visit_function_definition_expressions(node)
            self._nested(node.name, node.body)

        def _visit_function_definition_expressions(
            self,
            node: ast.FunctionDef | ast.AsyncFunctionDef,
        ) -> None:
            for decorator in node.decorator_list:
                self.visit(decorator)
            arguments = node.args
            for argument in (
                *arguments.posonlyargs,
                *arguments.args,
                *arguments.kwonlyargs,
            ):
                if argument.annotation is not None:
                    self.visit(argument.annotation)
            for argument in (arguments.vararg, arguments.kwarg):
                if argument is not None and argument.annotation is not None:
                    self.visit(argument.annotation)
            for default in (*arguments.defaults, *arguments.kw_defaults):
                if default is not None:
                    self.visit(default)
            if node.returns is not None:
                self.visit(node.returns)
            for type_parameter in getattr(node, "type_params", ()):  # Python 3.12+
                self.visit(type_parameter)

        def visit_Call(self, node: ast.Call) -> None:
            bounded = byte_range(node.func)
            spelling, qualifier = _python_name(node.func, source)
            if bounded is not None and spelling:
                start, end, line = bounded
                constructs.append(
                    SourceConstruct(
                        relative,
                        "calls",
                        "calls",
                        self.owner,
                        spelling,
                        qualifier,
                        start,
                        end,
                        line,
                    )
                )
            self.generic_visit(node)

        def visit_Import(self, node: ast.Import) -> None:
            for alias in node.names:
                bounded = byte_range(alias)
                if bounded is None:
                    continue
                start, end, line = bounded
                constructs.append(
                    SourceConstruct(
                        relative,
                        "imports",
                        "imports",
                        self.owner,
                        alias.name,
                        None,
                        start,
                        end,
                        line,
                    )
                )

        def visit_ImportFrom(self, node: ast.ImportFrom) -> None:
            module_name = "." * node.level + (node.module or "")
            for alias in node.names:
                bounded = byte_range(alias)
                if bounded is None:
                    continue
                start, end, line = bounded
                qualified = f"{module_name}.{alias.name}".strip(".")
                constructs.append(
                    SourceConstruct(
                        relative,
                        "imports",
                        "imports",
                        self.owner,
                        qualified or alias.name,
                        module_name or None,
                        start,
                        end,
                        line,
                    )
                )

    Visitor().visit(tree)
    return tuple(sorted(set(constructs), key=_source_construct_key))


_PYTHON_FRAMEWORK_MAX_FILES = 20_000
_PYTHON_FRAMEWORK_MAX_FILE_BYTES = 8 * 1024 * 1024
_PYTHON_FRAMEWORK_MAX_INCLUDE_TARGETS = 100_000
_PYTHON_ROUTE_METHODS = frozenset(
    {
        "route",
        "api_route",
        "get",
        "post",
        "put",
        "patch",
        "delete",
        "options",
        "head",
        "websocket",
        "websocket_route",
    }
)
_PYTHON_RECEIVER_CONSTRUCTORS = {
    "celery.Celery": "celery-python",
    "fastapi.FastAPI": "fastapi-python",
    "fastapi.APIRouter": "fastapi-python",
    "flask.Flask": "flask-python",
    "flask.Blueprint": "flask-python",
    "rest_framework.routers.DefaultRouter": "django-rest-framework-python",
    "rest_framework.routers.SimpleRouter": "django-rest-framework-python",
    "starlette.applications.Starlette": "starlette-python",
    "starlette.routing.Router": "starlette-python",
}


def _python_dotted_name(node: ast.AST | None) -> str | None:
    if isinstance(node, ast.Name):
        return node.id
    if isinstance(node, ast.Attribute):
        owner = _python_dotted_name(node.value)
        return f"{owner}.{node.attr}" if owner else None
    return None


@dataclass(frozen=True)
class _PythonDjangoIncludeMount:
    source_file: str
    owner_qualified_name: str
    target_reference: str
    start_byte: int
    end_byte: int
    start_line: int


@dataclass(frozen=True)
class _PythonDjangoUrlModule:
    module: str
    source_file: str
    pattern_ranges: frozenset[tuple[int, int]]
    include_mounts: tuple[_PythonDjangoIncludeMount, ...]


def _python_absolute_import_module(
    current_module: str,
    package_module: bool,
    imported_module: str,
    level: int,
) -> str | None:
    if level == 0:
        return imported_module or None
    package = current_module if package_module else current_module.rpartition(".")[0]
    parts = package.split(".") if package else []
    ascend = level - 1
    if ascend > len(parts):
        return None
    prefix = parts[: len(parts) - ascend]
    suffix = imported_module.split(".") if imported_module else []
    resolved = ".".join((*prefix, *suffix))
    return resolved or None


def _python_django_url_module(
    root: Path,
    path: Path,
) -> _PythonDjangoUrlModule | None:
    """Index exact, module-level Django URL-list includes without executing code."""

    try:
        raw = path.read_bytes()
        if len(raw) > _PYTHON_FRAMEWORK_MAX_FILE_BYTES:
            return None
        bom = len(tokenize.BOM_UTF8) if raw.startswith(tokenize.BOM_UTF8) else 0
        source = raw.decode("utf-8-sig")
        tree = ast.parse(source, filename=str(path), type_comments=True)
    except (OSError, SyntaxError, UnicodeError, ValueError):
        return None

    encoded_lines = [line.encode("utf-8") for line in source.splitlines(keepends=True)]
    line_offsets: list[int] = []
    offset = bom
    for line in encoded_lines:
        line_offsets.append(offset)
        offset += len(line)

    def byte_range(node: ast.AST) -> tuple[int, int, int] | None:
        start_line = getattr(node, "lineno", None)
        end_line = getattr(node, "end_lineno", None)
        start_column = getattr(node, "col_offset", None)
        end_column = getattr(node, "end_col_offset", None)
        if (
            not isinstance(start_line, int)
            or not isinstance(end_line, int)
            or not isinstance(start_column, int)
            or not isinstance(end_column, int)
            or start_line < 1
            or end_line < start_line
            or end_line > len(line_offsets)
        ):
            return None
        start = line_offsets[start_line - 1] + start_column
        end = line_offsets[end_line - 1] + end_column
        if start < bom or end <= start or end > len(raw):
            return None
        return start, end, start_line

    module = _python_module(root, path)
    package_module = path.stem == "__init__"
    relative = path.relative_to(root).as_posix()
    bindings: dict[str, str] = {}
    imported_modules: set[str] = set()
    for statement in tree.body:
        if isinstance(statement, ast.Import):
            for alias in statement.names:
                imported_modules.add(alias.name)
                if alias.asname is not None:
                    bindings[alias.asname] = alias.name
                else:
                    bindings[alias.name.split(".", 1)[0]] = alias.name.split(".", 1)[0]
        elif isinstance(statement, ast.ImportFrom):
            imported = _python_absolute_import_module(
                module,
                package_module,
                statement.module or "",
                statement.level,
            )
            if imported is None:
                continue
            for alias in statement.names:
                if alias.name == "*":
                    continue
                bindings[alias.asname or alias.name] = f"{imported}.{alias.name}"

    for statement in tree.body:
        shadowed: list[str] = []
        if isinstance(statement, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
            shadowed.append(statement.name)
        elif isinstance(statement, ast.Assign):
            shadowed.extend(
                target.id for target in statement.targets if isinstance(target, ast.Name)
            )
        elif isinstance(statement, ast.AnnAssign) and isinstance(statement.target, ast.Name):
            shadowed.append(statement.target.id)
        for name in shadowed:
            bindings.pop(name, None)

    def resolved_name(node: ast.AST | None) -> str | None:
        dotted = _python_dotted_name(node)
        if dotted is None:
            return None
        head, separator, tail = dotted.partition(".")
        bound = bindings.get(head)
        if bound is None:
            return dotted
        return f"{bound}.{tail}" if separator else bound

    definitions: dict[str, list[ast.AST]] = {}
    pattern_roots: list[ast.AST] = []
    for statement in tree.body:
        target: ast.Name | None = None
        value: ast.AST | None = None
        if isinstance(statement, ast.Assign) and len(statement.targets) == 1:
            candidate = statement.targets[0]
            if isinstance(candidate, ast.Name):
                target = candidate
                value = statement.value
        elif isinstance(statement, ast.AnnAssign) and isinstance(statement.target, ast.Name):
            target = statement.target
            value = statement.value
        if target is not None and value is not None:
            definitions.setdefault(target.id, []).append(value)
            if target.id == "urlpatterns":
                pattern_roots = [value]
        elif (
            isinstance(statement, ast.AugAssign)
            and isinstance(statement.target, ast.Name)
            and statement.target.id == "urlpatterns"
            and isinstance(statement.op, ast.Add)
        ):
            pattern_roots.append(statement.value)

    unique_definitions = {
        name: values[0]
        for name, values in definitions.items()
        if name != "urlpatterns" and len(values) == 1
    }

    def pattern_calls(node: ast.AST, active: frozenset[str]) -> tuple[ast.Call, ...]:
        if isinstance(node, ast.Call):
            called = resolved_name(node.func)
            return (
                (node,)
                if called
                in {
                    "django.urls.path",
                    "django.urls.re_path",
                    "django.conf.urls.url",
                }
                else ()
            )
        if isinstance(node, (ast.List, ast.Tuple)):
            return tuple(
                call
                for element in node.elts
                for call in pattern_calls(element, active)
            )
        if isinstance(node, ast.BinOp) and isinstance(node.op, ast.Add):
            return (*pattern_calls(node.left, active), *pattern_calls(node.right, active))
        if isinstance(node, ast.Name) and node.id not in active:
            definition = unique_definitions.get(node.id)
            if definition is not None:
                return pattern_calls(definition, active | {node.id})
        return ()

    calls = tuple(
        call for root_node in pattern_roots for call in pattern_calls(root_node, frozenset())
    )
    if len(calls) > _PYTHON_FRAMEWORK_MAX_INCLUDE_TARGETS:
        raise RuntimeError(
            "Python Django URL pattern count "
            f"{len(calls)} exceeds {_PYTHON_FRAMEWORK_MAX_INCLUDE_TARGETS}"
        )

    def handler_argument(call: ast.Call, position: int, keyword: str) -> ast.AST | None:
        for item in call.keywords:
            if item.arg == keyword:
                return item.value
        return call.args[position] if len(call.args) > position else None

    def static_module(node: ast.AST | None) -> str | None:
        if isinstance(node, ast.Constant) and isinstance(node.value, str):
            candidate = node.value
        elif isinstance(node, (ast.Tuple, ast.List)) and node.elts:
            return static_module(node.elts[0])
        else:
            candidate = resolved_name(node)
            if candidate is None:
                return None
            head = (_python_dotted_name(node) or "").partition(".")[0]
            if head not in bindings and candidate not in imported_modules:
                return None
        if not candidate or any(not part.isidentifier() for part in candidate.split(".")):
            return None
        return candidate

    pattern_ranges: set[tuple[int, int]] = set()
    mounts: list[_PythonDjangoIncludeMount] = []
    for call in calls:
        bounded = byte_range(call)
        if bounded is None:
            continue
        start, end, line = bounded
        pattern_ranges.add((start, end))
        handler = handler_argument(call, 1, "view")
        if not isinstance(handler, ast.Call) or resolved_name(handler.func) not in {
            "django.urls.include",
            "django.conf.urls.include",
        }:
            continue
        reference = static_module(handler_argument(handler, 0, "arg"))
        if reference is None:
            continue
        mounts.append(
            _PythonDjangoIncludeMount(
                relative,
                module,
                reference,
                start,
                end,
                line,
            )
        )

    return _PythonDjangoUrlModule(
        module,
        relative,
        frozenset(pattern_ranges),
        tuple(
            sorted(
                set(mounts),
                key=lambda mount: (
                    mount.source_file,
                    mount.start_byte,
                    mount.end_byte,
                    mount.target_reference,
                ),
            )
        ),
    )


def _python_framework_constructs(
    root: Path,
    path: Path,
) -> tuple[SourceConstruct, ...] | None:
    """Collect exact Python framework relations without importing corpus code."""

    try:
        raw = path.read_bytes()
        if len(raw) > _PYTHON_FRAMEWORK_MAX_FILE_BYTES:
            return None
        bom = len(tokenize.BOM_UTF8) if raw.startswith(tokenize.BOM_UTF8) else 0
        source = raw.decode("utf-8-sig")
        tree = ast.parse(source, filename=str(path), type_comments=True)
    except (OSError, SyntaxError, UnicodeError, ValueError):
        return None

    encoded_lines = [line.encode("utf-8") for line in source.splitlines(keepends=True)]
    line_offsets: list[int] = []
    offset = bom
    for line in encoded_lines:
        line_offsets.append(offset)
        offset += len(line)

    def byte_range(node: ast.AST) -> tuple[int, int, int] | None:
        start_line = getattr(node, "lineno", None)
        end_line = getattr(node, "end_lineno", None)
        start_column = getattr(node, "col_offset", None)
        end_column = getattr(node, "end_col_offset", None)
        if (
            not isinstance(start_line, int)
            or not isinstance(end_line, int)
            or not isinstance(start_column, int)
            or not isinstance(end_column, int)
            or start_line < 1
            or end_line < start_line
            or end_line > len(line_offsets)
        ):
            return None
        start = line_offsets[start_line - 1] + start_column
        end = line_offsets[end_line - 1] + end_column
        if start < bom or end <= start or end > len(raw):
            return None
        return start, end, start_line

    def definition_name_range(
        node: ast.FunctionDef | ast.AsyncFunctionDef,
    ) -> tuple[int, int, int] | None:
        if node.lineno < 1 or node.lineno > len(line_offsets):
            return None
        line_start = line_offsets[node.lineno - 1]
        line_end = (
            line_offsets[node.lineno]
            if node.lineno < len(line_offsets)
            else len(raw)
        )
        search_start = line_start + node.col_offset
        name = node.name.encode("utf-8")
        start = raw.find(name, search_start, line_end)
        if start < 0:
            return None
        end = start + len(name)
        before = raw[start - 1 : start] if start else b""
        after = raw[end : end + 1]
        identifier = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_"
        if before and before in identifier or after and after in identifier:
            return None
        return start, end, node.lineno

    bindings: dict[str, str] = {}
    for statement in tree.body:
        if isinstance(statement, ast.Import):
            for alias in statement.names:
                bindings[alias.asname or alias.name.split(".", 1)[0]] = alias.name
        elif isinstance(statement, ast.ImportFrom):
            module = "." * statement.level + (statement.module or "")
            if statement.level == 0:
                for alias in statement.names:
                    if alias.name != "*":
                        bindings[alias.asname or alias.name] = f"{module}.{alias.name}"

    for statement in tree.body:
        shadowed: list[str] = []
        if isinstance(statement, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
            shadowed.append(statement.name)
        elif isinstance(statement, ast.Assign):
            shadowed.extend(
                target.id for target in statement.targets if isinstance(target, ast.Name)
            )
        elif isinstance(statement, ast.AnnAssign) and isinstance(statement.target, ast.Name):
            shadowed.append(statement.target.id)
        for name in shadowed:
            if not (
                isinstance(statement, (ast.Assign, ast.AnnAssign))
                and isinstance(statement.value, ast.Call)
                and _python_dotted_name(statement.value.func) == name
            ):
                bindings.pop(name, None)

    def resolved_name(node: ast.AST | None) -> str | None:
        dotted = _python_dotted_name(node)
        if dotted is None:
            return None
        head, separator, tail = dotted.partition(".")
        bound = bindings.get(head)
        if bound is None:
            return dotted
        return f"{bound}.{tail}" if separator else bound

    receivers: dict[str, str] = {}
    for statement in tree.body:
        if not isinstance(statement, (ast.Assign, ast.AnnAssign)):
            continue
        value = statement.value
        targets = statement.targets if isinstance(statement, ast.Assign) else [statement.target]
        for target in targets:
            if not isinstance(target, ast.Name):
                continue
            framework_pack = (
                _PYTHON_RECEIVER_CONSTRUCTORS.get(resolved_name(value.func) or "")
                if isinstance(value, ast.Call)
                else None
            )
            if framework_pack is None:
                receivers.pop(target.id, None)
            else:
                receivers[target.id] = framework_pack

    declarations: dict[str, list[tuple[int, int, int]]] = {}
    for node in ast.walk(tree):
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
            bounded = definition_name_range(node)
            if bounded is not None:
                declarations.setdefault(node.name, []).append(bounded)

    relative = path.relative_to(root).as_posix()
    module = _python_module(root, path)
    constructs: list[SourceConstruct] = []

    def add(
        relation: str,
        capability: str,
        owner: str,
        target: str,
        bounded: tuple[int, int, int] | None,
        framework_pack: str,
        qualifier: str | None = None,
    ) -> None:
        if bounded is None:
            return
        start, end, line = bounded
        constructs.append(
            SourceConstruct(
                relative,
                relation,
                capability,
                owner,
                target,
                qualifier,
                start,
                end,
                line,
                framework_pack,
            )
        )

    def local_declaration(name: str) -> tuple[int, int, int] | None:
        values = declarations.get(name, ())
        return values[0] if len(values) == 1 else None

    def handler_argument(call: ast.Call, position: int, keyword: str) -> ast.AST | None:
        for item in call.keywords:
            if item.arg == keyword:
                return item.value
        return call.args[position] if len(call.args) > position else None

    def keyword_argument(call: ast.Call, keyword: str) -> ast.AST | None:
        return next((item.value for item in call.keywords if item.arg == keyword), None)

    def static_string(node: ast.AST | None) -> str | None:
        return node.value if isinstance(node, ast.Constant) and isinstance(node.value, str) else None

    def decorator_range(node: ast.AST) -> tuple[int, int, int] | None:
        bounded = byte_range(node)
        if bounded is None:
            return None
        start, end, line = bounded
        if start > bom and raw[start - 1 : start] == b"@":
            start -= 1
        return start, end, line

    classes_by_name: dict[str, list[ast.ClassDef]] = {}
    for candidate in ast.walk(tree):
        if isinstance(candidate, ast.ClassDef):
            classes_by_name.setdefault(candidate.name, []).append(candidate)
    unique_classes = {
        name: values[0] for name, values in classes_by_name.items() if len(values) == 1
    }
    module_classes_by_name: dict[str, list[ast.ClassDef]] = {}
    for candidate in tree.body:
        if isinstance(candidate, ast.ClassDef):
            module_classes_by_name.setdefault(candidate.name, []).append(candidate)
    unique_module_classes = {
        name: values[0]
        for name, values in module_classes_by_name.items()
        if len(values) == 1
    }

    def descendant_class_names(
        external_bases: set[str],
        definitions: dict[str, ast.ClassDef] | None = None,
    ) -> tuple[set[str], set[str]]:
        definitions = unique_classes if definitions is None else definitions
        selected: set[str] = set()
        direct: set[str] = set()
        for _ in range(len(definitions) + 1):
            previous = len(selected)
            for name, definition in definitions.items():
                bases = {resolved_name(base) for base in definition.bases}
                if bases & external_bases:
                    selected.add(name)
                    direct.add(name)
                elif any(
                    isinstance(base, ast.Name) and base.id in selected
                    for base in definition.bases
                ):
                    selected.add(name)
            if len(selected) == previous:
                break
        return selected, direct

    django_models, _ = descendant_class_names({"django.db.models.Model"})
    django_managers, _ = descendant_class_names({"django.db.models.Manager"})
    drf_serializers, _ = descendant_class_names(
        {
            "rest_framework.serializers.Serializer",
            "rest_framework.serializers.ModelSerializer",
        }
    )
    drf_viewsets, _ = descendant_class_names(
        {
            "rest_framework.viewsets.ViewSet",
            "rest_framework.viewsets.GenericViewSet",
            "rest_framework.viewsets.ModelViewSet",
            "rest_framework.viewsets.ReadOnlyModelViewSet",
        }
    )
    module_drf_viewsets, _ = descendant_class_names(
        {
            "rest_framework.viewsets.ViewSet",
            "rest_framework.viewsets.GenericViewSet",
            "rest_framework.viewsets.ModelViewSet",
            "rest_framework.viewsets.ReadOnlyModelViewSet",
        },
        unique_module_classes,
    )
    pydantic_models, _ = descendant_class_names({"pydantic.BaseModel"})
    sqlalchemy_descendants, sqlalchemy_bases = descendant_class_names(
        {"sqlalchemy.orm.DeclarativeBase"}
    )
    sqlalchemy_models = sqlalchemy_descendants - sqlalchemy_bases

    def assignment_parts(
        statement: ast.stmt,
    ) -> tuple[ast.Name, ast.AST] | None:
        if isinstance(statement, ast.AnnAssign):
            if isinstance(statement.target, ast.Name) and statement.value is not None:
                return statement.target, statement.value
            return None
        if isinstance(statement, ast.Assign) and len(statement.targets) == 1:
            target = statement.targets[0]
            if isinstance(target, ast.Name):
                return target, statement.value
        return None

    def nested_class(definition: ast.ClassDef, name: str) -> ast.ClassDef | None:
        matches = [
            statement
            for statement in definition.body
            if isinstance(statement, ast.ClassDef) and statement.name == name
        ]
        return matches[0] if len(matches) == 1 else None

    def local_model_from_expression(node: ast.AST | None, models: set[str]) -> str | None:
        return node.id if isinstance(node, ast.Name) and node.id in models else None

    def local_models_in_annotation(node: ast.AST | None, models: set[str]) -> set[str]:
        if node is None:
            return set()
        return {
            candidate.id
            for candidate in ast.walk(node)
            if isinstance(candidate, ast.Name) and candidate.id in models
        }

    def exact_static_references(node: ast.AST) -> tuple[str, ...]:
        if isinstance(node, ast.Name):
            bound = bindings.get(node.id)
            if bound is not None:
                return (bound,)
            if node.id in unique_classes or len(declarations.get(node.id, ())) == 1:
                return (node.id,)
            return ()
        if isinstance(node, ast.Attribute):
            dotted = _python_dotted_name(node)
            if dotted is None or dotted.partition(".")[0] not in bindings:
                return ()
            resolved = resolved_name(node)
            return (resolved,) if resolved is not None else ()
        if isinstance(node, (ast.List, ast.Tuple, ast.Set)):
            values = {
                reference
                for element in node.elts
                for reference in exact_static_references(element)
            }
            return tuple(sorted(values))
        return ()

    def celery_task_info(
        decorator: ast.AST,
        function_name: str,
    ) -> tuple[str, str | None] | None:
        call = decorator if isinstance(decorator, ast.Call) else None
        target = call.func if call is not None else decorator
        called = resolved_name(target)
        exact = called == "celery.shared_task"
        if not exact and isinstance(target, ast.Attribute) and target.attr == "task":
            receiver = _python_dotted_name(target.value)
            exact = receivers.get(receiver or "") == "celery-python"
        if not exact:
            return None
        configured = static_string(keyword_argument(call, "name")) if call is not None else None
        queue = static_string(keyword_argument(call, "queue")) if call is not None else None
        return configured or ".".join(part for part in (module, function_name) if part), queue

    celery_tasks: dict[str, tuple[str, str | None]] = {}
    for candidate in ast.walk(tree):
        if not isinstance(candidate, (ast.FunctionDef, ast.AsyncFunctionDef)):
            continue
        infos = [
            info
            for decorator in candidate.decorator_list
            if (info := celery_task_info(decorator, candidate.name)) is not None
        ]
        if len(infos) == 1 and len(declarations.get(candidate.name, ())) == 1:
            celery_tasks[candidate.name] = infos[0]

    mounted_drf_routers: set[str] = set()
    for statement in tree.body:
        value: ast.AST | None = None
        targets: tuple[ast.AST, ...] = ()
        if isinstance(statement, ast.Assign):
            value = statement.value
            targets = tuple(statement.targets)
        elif isinstance(statement, ast.AnnAssign):
            value = statement.value
            targets = (statement.target,)
        elif isinstance(statement, ast.AugAssign):
            value = statement.value
            targets = (statement.target,)
        if value is None or not any(
            isinstance(target, ast.Name) and target.id == "urlpatterns"
            for target in targets
        ):
            continue
        for candidate in ast.walk(value):
            if (
                isinstance(candidate, ast.Attribute)
                and candidate.attr == "urls"
                and isinstance(candidate.value, ast.Name)
                and receivers.get(candidate.value.id)
                == "django-rest-framework-python"
            ):
                mounted_drf_routers.add(candidate.value.id)

    def drf_viewset_route_targets(viewset: str) -> tuple[str, ...]:
        methods: set[str] = set()
        standard = {
            "list",
            "create",
            "retrieve",
            "update",
            "partial_update",
            "destroy",
        }
        definition = unique_module_classes.get(viewset) or unique_classes[viewset]
        for statement in definition.body:
            if not isinstance(statement, (ast.FunctionDef, ast.AsyncFunctionDef)):
                continue
            if statement.name in standard:
                methods.add(statement.name)
                continue
            if any(
                resolved_name(
                    decorator.func if isinstance(decorator, ast.Call) else decorator
                )
                == "rest_framework.decorators.action"
                for decorator in statement.decorator_list
            ):
                methods.add(statement.name)
        return tuple(sorted(methods))

    def collect_class_relations() -> None:
        django_relationships = {
            "django.db.models.ForeignKey",
            "django.db.models.ManyToManyField",
            "django.db.models.OneToOneField",
        }
        for model_name in sorted(django_models):
            definition = unique_classes[model_name]
            for statement in definition.body:
                parts = assignment_parts(statement)
                if parts is None:
                    continue
                field, value = parts
                if not isinstance(value, ast.Call):
                    continue
                constructor = resolved_name(value.func)
                if constructor in django_relationships:
                    related = local_model_from_expression(
                        handler_argument(value, 0, "to"), django_models
                    )
                    if related is not None:
                        add(
                            "depends_on",
                            "persistence",
                            model_name,
                            related,
                            byte_range(field),
                            "django-python",
                        )
                elif (
                    isinstance(value.func, ast.Name)
                    and value.func.id in django_managers
                ):
                    add(
                        "depends_on",
                        "persistence",
                        model_name,
                        value.func.id,
                        byte_range(statement),
                        "django-python",
                    )

        for model_name in sorted(sqlalchemy_models):
            definition = unique_classes[model_name]
            for statement in definition.body:
                parts = assignment_parts(statement)
                if parts is None:
                    continue
                field, value = parts
                if field.id == "__tablename__":
                    table = static_string(value)
                    if table:
                        add(
                            "maps_to",
                            "persistence",
                            model_name,
                            table,
                            byte_range(value),
                            "sqlalchemy-python",
                        )
                    continue
                if not isinstance(value, ast.Call):
                    continue
                if resolved_name(value.func) != "sqlalchemy.orm.relationship":
                    continue
                related = local_model_from_expression(
                    handler_argument(value, 0, "argument"), sqlalchemy_models
                )
                if related is None and isinstance(statement, ast.AnnAssign):
                    candidates = local_models_in_annotation(
                        statement.annotation, sqlalchemy_models
                    )
                    related = next(iter(candidates)) if len(candidates) == 1 else None
                if related is not None:
                    add(
                        "depends_on",
                        "data_modeling",
                        model_name,
                        related,
                        byte_range(statement),
                        "sqlalchemy-python",
                    )

        for serializer_name in sorted(drf_serializers):
            meta = nested_class(unique_classes[serializer_name], "Meta")
            if meta is None:
                continue
            for statement in meta.body:
                parts = assignment_parts(statement)
                if parts is None or parts[0].id != "model":
                    continue
                target = local_model_from_expression(parts[1], django_models)
                if target is not None:
                    add(
                        "depends_on",
                        "dependency_injection",
                        serializer_name,
                        target,
                        byte_range(parts[1]),
                        "django-rest-framework-python",
                    )

        viewset_fields = {
            "serializer_class",
            "permission_classes",
            "authentication_classes",
            "filter_backends",
            "throttle_classes",
        }
        for viewset_name in sorted(drf_viewsets):
            for statement in unique_classes[viewset_name].body:
                parts = assignment_parts(statement)
                if parts is None or parts[0].id not in viewset_fields:
                    continue
                for target in exact_static_references(parts[1]):
                    add(
                        "depends_on",
                        "dependency_injection",
                        viewset_name,
                        target,
                        byte_range(parts[1]),
                        "django-rest-framework-python",
                    )

    collect_class_relations()

    class Visitor(ast.NodeVisitor):
        def __init__(self) -> None:
            self.owners = [module]

        @property
        def owner(self) -> str:
            return ".".join(part for part in self.owners if part)

        def _visit_function(
            self,
            node: ast.FunctionDef | ast.AsyncFunctionDef,
        ) -> None:
            bounded = definition_name_range(node)
            for decorator in node.decorator_list:
                if isinstance(decorator, ast.Call) and isinstance(
                    decorator.func, ast.Attribute
                ):
                    receiver = _python_dotted_name(decorator.func.value)
                    framework_pack = receivers.get(receiver or "")
                    if (
                        framework_pack is not None
                        and decorator.func.attr in _PYTHON_ROUTE_METHODS
                    ):
                        add(
                            "routes_to",
                            "http_routes",
                            self.owner,
                            node.name,
                            bounded,
                            framework_pack,
                            receiver,
                        )
                    if (
                        framework_pack == "fastapi-python"
                        and decorator.func.attr in _PYTHON_ROUTE_METHODS
                    ):
                        route_models = local_models_in_annotation(
                            node.returns, pydantic_models
                        )
                        for parameter in (
                            *node.args.posonlyargs,
                            *node.args.args,
                            *node.args.kwonlyargs,
                        ):
                            route_models.update(
                                local_models_in_annotation(
                                    parameter.annotation, pydantic_models
                                )
                            )
                        explicit_model = local_model_from_expression(
                            keyword_argument(decorator, "response_model"),
                            pydantic_models,
                        )
                        if explicit_model is not None:
                            route_models.add(explicit_model)
                        for model in sorted(route_models):
                            add(
                                "depends_on",
                                "data_modeling",
                                self.owner,
                                model,
                                decorator_range(decorator),
                                "pydantic-python",
                            )

                task = celery_task_info(decorator, node.name)
                if task is not None:
                    task_name, queue = task
                    add(
                        "schedules",
                        "scheduling",
                        self.owner,
                        task_name,
                        decorator_range(decorator),
                        "celery-python",
                    )
                    add(
                        "triggers",
                        "messaging",
                        self.owner,
                        node.name,
                        decorator_range(decorator),
                        "celery-python",
                    )
                    if queue is not None:
                        add(
                            "consumes",
                            "messaging",
                            self.owner,
                            queue,
                            decorator_range(decorator),
                            "celery-python",
                        )

                if (
                    isinstance(decorator, ast.Call)
                    and resolved_name(decorator.func) == "django.dispatch.receiver"
                ):
                    signal_node = handler_argument(decorator, 0, "signal")
                    signal = resolved_name(signal_node)
                    if signal is not None and signal.startswith(
                        "django.db.models.signals."
                    ):
                        add(
                            "subscribes",
                            "messaging",
                            self.owner,
                            signal,
                            decorator_range(decorator),
                            "django-python",
                        )
                        sender_node = keyword_argument(decorator, "sender")
                        sender = local_model_from_expression(sender_node, django_models)
                        if sender is not None:
                            add(
                                "depends_on",
                                "persistence",
                                self.owner,
                                sender,
                                byte_range(sender_node),
                                "django-python",
                            )
            self.owners.append(node.name)
            self.generic_visit(node)
            self.owners.pop()

        def visit_FunctionDef(self, node: ast.FunctionDef) -> None:  # noqa: N802
            self._visit_function(node)

        def visit_AsyncFunctionDef(self, node: ast.AsyncFunctionDef) -> None:  # noqa: N802
            self._visit_function(node)

        def visit_Call(self, node: ast.Call) -> None:  # noqa: N802
            called = resolved_name(node.func)
            if (
                isinstance(node.func, ast.Attribute)
                and node.func.attr == "register"
                and isinstance(node.func.value, ast.Name)
                and node.func.value.id in mounted_drf_routers
                and static_string(keyword_argument(node, "basename")) is not None
            ):
                viewset_node = handler_argument(node, 1, "viewset")
                if (
                    isinstance(viewset_node, ast.Name)
                    and viewset_node.id in module_drf_viewsets
                ):
                    for method in drf_viewset_route_targets(viewset_node.id):
                        add(
                            "routes_to",
                            "http_routes",
                            self.owner,
                            method,
                            byte_range(node),
                            "django-rest-framework-python",
                            node.func.value.id,
                        )
            framework_pack: str | None = None
            handler: ast.AST | None = None
            handler_anchor: tuple[int, int, int] | None = None
            if called in {"starlette.routing.Route", "starlette.routing.WebSocketRoute"}:
                framework_pack = "starlette-python"
                handler = handler_argument(node, 1, "endpoint")
            elif called in {
                "django.urls.path",
                "django.urls.re_path",
                "django.conf.urls.url",
            }:
                framework_pack = "django-python"
                handler = handler_argument(node, 1, "view")
                handler_anchor = byte_range(node)
            elif isinstance(node.func, ast.Attribute):
                receiver = _python_dotted_name(node.func.value)
                candidate_pack = receivers.get(receiver or "")
                if candidate_pack in {"fastapi-python", "starlette-python"} and node.func.attr in {
                    "add_api_route",
                    "add_route",
                    "add_websocket_route",
                }:
                    framework_pack = candidate_pack
                    handler = handler_argument(node, 1, "endpoint")
                elif candidate_pack == "flask-python" and node.func.attr == "add_url_rule":
                    framework_pack = candidate_pack
                    handler = handler_argument(node, 1, "view_func")
            route_target: str | None = None
            target_node = handler.func if isinstance(handler, ast.Call) else handler
            if framework_pack == "django-python" and isinstance(handler, ast.Call):
                handler_called = resolved_name(handler.func)
                if handler_called in {
                    "django.urls.include",
                    "django.conf.urls.include",
                }:
                    target_node = None
                elif isinstance(handler.func, ast.Attribute) and handler.func.attr == "as_view":
                    target_node = handler.func.value
            if isinstance(target_node, (ast.Name, ast.Attribute)):
                route_target = _python_dotted_name(target_node)
            if framework_pack is not None and route_target is not None:
                add(
                    "routes_to",
                    "http_routes",
                    self.owner,
                    route_target,
                    handler_anchor
                    or (
                        local_declaration(route_target)
                        if isinstance(handler, ast.Name)
                        else byte_range(target_node)
                    ),
                    framework_pack,
                )

            if called in {"fastapi.Depends", "fastapi.Security"}:
                provider = handler_argument(node, 0, "dependency")
                if isinstance(provider, (ast.Name, ast.Attribute)):
                    target = _python_dotted_name(provider)
                    if target:
                        add(
                            "depends_on",
                            "dependency_injection",
                            self.owner,
                            target,
                            byte_range(node),
                            "fastapi-python",
                        )

            if isinstance(node.func, ast.Attribute):
                member = node.func.attr
                receiver_node = node.func.value
                if (
                    isinstance(receiver_node, ast.Name)
                    and receiver_node.id in celery_tasks
                    and member in {"delay", "apply_async", "s", "si", "signature"}
                ):
                    add(
                        "triggers",
                        "messaging",
                        self.owner,
                        receiver_node.id,
                        byte_range(node),
                        "celery-python",
                    )
                    queue = static_string(keyword_argument(node, "queue"))
                    if queue is not None:
                        add(
                            "produces",
                            "messaging",
                            self.owner,
                            queue,
                            byte_range(node),
                            "celery-python",
                        )
                elif member == "send_task":
                    receiver = _python_dotted_name(receiver_node)
                    if receivers.get(receiver or "") == "celery-python":
                        task_name = static_string(handler_argument(node, 0, "name"))
                        if task_name is not None:
                            add(
                                "produces",
                                "messaging",
                                self.owner,
                                task_name,
                                byte_range(node),
                                "celery-python",
                            )
                        queue = static_string(keyword_argument(node, "queue"))
                        if queue is not None:
                            add(
                                "produces",
                                "messaging",
                                self.owner,
                                queue,
                                byte_range(node),
                                "celery-python",
                            )
            self.generic_visit(node)

    Visitor().visit(tree)
    return tuple(sorted(set(constructs), key=_source_construct_key))


def _qualification_glob_matches(relative: str, pattern: str) -> bool:
    if fnmatch.fnmatchcase(relative, pattern):
        return True
    return "**/" in pattern and fnmatch.fnmatchcase(
        relative,
        pattern.replace("**/", ""),
    )


def _python_framework_inventory(
    root: Path,
    include_globs: tuple[str, ...],
    exclude_globs: tuple[str, ...],
) -> SourceConstructInventory:
    paths = sorted(path for path in root.rglob("*.py") if path.is_file())
    if len(paths) > _PYTHON_FRAMEWORK_MAX_FILES:
        raise RuntimeError(
            f"Python framework source file count {len(paths)} exceeds {_PYTHON_FRAMEWORK_MAX_FILES}"
        )
    constructs: list[SourceConstruct] = []
    django_modules: list[_PythonDjangoUrlModule] = []
    rejected: list[str] = []
    scanned = 0
    parsed = 0
    for path in paths:
        resolved = path.resolve()
        try:
            relative = resolved.relative_to(root).as_posix()
        except ValueError:
            rejected.append(path.relative_to(root).as_posix())
            continue
        if include_globs and not any(
            _qualification_glob_matches(relative, pattern) for pattern in include_globs
        ):
            continue
        if any(_qualification_glob_matches(relative, pattern) for pattern in exclude_globs):
            continue
        scanned += 1
        extracted = _python_framework_constructs(root, resolved)
        if extracted is None:
            rejected.append(relative)
        else:
            parsed += 1
            constructs.extend(extracted)
            django_module = _python_django_url_module(root, resolved)
            if django_module is not None:
                django_modules.append(django_module)

    module_counts: dict[str, int] = {}
    for django_module in django_modules:
        module_counts[django_module.module] = module_counts.get(django_module.module, 0) + 1
    modules = {
        django_module.module: django_module
        for django_module in django_modules
        if module_counts[django_module.module] == 1
    }
    direct_targets: dict[str, set[tuple[str, str]]] = {
        module: set() for module in modules
    }
    module_by_source = {
        django_module.source_file: django_module
        for django_module in modules.values()
    }
    for construct in constructs:
        django_module = module_by_source.get(construct.source_file)
        if (
            django_module is not None
            and construct.framework_pack == "django-python"
            and construct.relation == "routes_to"
            and (construct.start_byte, construct.end_byte)
            in django_module.pattern_ranges
        ):
            direct_targets[django_module.module].add(
                (django_module.module, construct.target_spelling)
            )

    def mounted_module(reference: str) -> str | None:
        if reference in modules:
            return reference
        for suffix in (".urlpatterns", ".urls"):
            if reference.endswith(suffix):
                candidate = reference[: -len(suffix)]
                if candidate in modules:
                    return candidate
        return None

    memoized_targets: dict[str, frozenset[tuple[str, str]]] = {}

    def downstream_targets(
        module: str,
        active: frozenset[str],
    ) -> frozenset[tuple[str, str]]:
        cached = memoized_targets.get(module)
        if cached is not None:
            return cached
        if module in active:
            return frozenset()
        selected = set(direct_targets[module])
        next_active = active | {module}
        for mount in modules[module].include_mounts:
            child = mounted_module(mount.target_reference)
            if child is not None:
                selected.update(downstream_targets(child, next_active))
            if len(selected) > _PYTHON_FRAMEWORK_MAX_INCLUDE_TARGETS:
                raise RuntimeError(
                    "Python Django included route target count exceeds "
                    f"{_PYTHON_FRAMEWORK_MAX_INCLUDE_TARGETS}"
                )
        bounded = frozenset(selected)
        memoized_targets[module] = bounded
        return bounded

    propagated = 0
    for module in sorted(modules):
        for mount in modules[module].include_mounts:
            child = mounted_module(mount.target_reference)
            if child is None:
                continue
            for target_module, target in sorted(
                downstream_targets(child, frozenset((module,)))
            ):
                constructs.append(
                    SourceConstruct(
                        mount.source_file,
                        "routes_to",
                        "http_routes",
                        mount.owner_qualified_name,
                        target,
                        target_module,
                        mount.start_byte,
                        mount.end_byte,
                        mount.start_line,
                        "django-python",
                    )
                )
                propagated += 1
                if propagated > _PYTHON_FRAMEWORK_MAX_INCLUDE_TARGETS:
                    raise RuntimeError(
                        "Python Django propagated route target count exceeds "
                        f"{_PYTHON_FRAMEWORK_MAX_INCLUDE_TARGETS}"
                    )
    return SourceConstructInventory(
        tuple(sorted(set(constructs), key=_source_construct_key)),
        scanned,
        parsed,
        tuple(sorted(rejected)),
    )


_TYPESCRIPT_ORACLE_SCHEMA = "compass.typescript-source-oracle/1"
_TYPESCRIPT_ORACLE_JSONL_SCHEMA = "compass.typescript-source-oracle-jsonl/3"
_TYPESCRIPT_ORACLE_PROVIDER = "typescript_compiler_api_5_9_3"
_TYPESCRIPT_ORACLE_SCRIPT = (
    Path(__file__).resolve().parents[1] / "oracles" / "typescript-source-oracle.mjs"
)
_TYPESCRIPT_ORACLE_TIMEOUT_SECONDS = 90.0
_TYPESCRIPT_ORACLE_OUTPUT_BYTES = 64 * 1024 * 1024
_TYPESCRIPT_ORACLE_MAX_TYPED_FACTS = 500_000
_RUBY_ORACLE_SCHEMA = "compass.ruby-source-oracle/1"
_RUBY_ORACLE_PROVIDER = "ruby_ripper_4_0_6"
_RUBY_ORACLE_SCRIPT = Path(__file__).resolve().parents[3] / "scripts" / "ruby_source_oracle.rb"
_RUBY_ORACLE_TIMEOUT_SECONDS = 600.0
_RUBY_ORACLE_OUTPUT_BYTES = 512 * 1024 * 1024


def _bounded_node_oracle(root: Path) -> bytes:
    """Run the independent compiler oracle with bounded pipes and duration."""

    if not _TYPESCRIPT_ORACLE_SCRIPT.is_file():
        raise RuntimeError(f"TypeScript source oracle is missing: {_TYPESCRIPT_ORACLE_SCRIPT}")
    command = (
        "node",
        str(_TYPESCRIPT_ORACLE_SCRIPT),
        "--root",
        str(root),
        "--jsonl",
    )
    process = subprocess.Popen(
        command,
        cwd=_TYPESCRIPT_ORACLE_SCRIPT.parents[3],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        shell=False,
    )
    assert process.stdout is not None
    assert process.stderr is not None
    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ, "stdout")
    selector.register(process.stderr, selectors.EVENT_READ, "stderr")
    output = bytearray()
    error = bytearray()
    started = time.monotonic()
    limit_error: str | None = None
    try:
        while selector.get_map():
            remaining = _TYPESCRIPT_ORACLE_TIMEOUT_SECONDS - (
                time.monotonic() - started
            )
            if remaining <= 0:
                limit_error = (
                    "TypeScript source oracle exceeded "
                    f"{_TYPESCRIPT_ORACLE_TIMEOUT_SECONDS:.0f}s"
                )
                process.kill()
                break
            events = selector.select(min(remaining, 0.25))
            for key, _ in events:
                chunk = os.read(key.fileobj.fileno(), 64 * 1024)
                if not chunk:
                    selector.unregister(key.fileobj)
                    key.fileobj.close()
                    continue
                if key.data == "stdout":
                    output.extend(chunk)
                    if len(output) > _TYPESCRIPT_ORACLE_OUTPUT_BYTES:
                        limit_error = (
                            "TypeScript source oracle output exceeds "
                            f"{_TYPESCRIPT_ORACLE_OUTPUT_BYTES} bytes"
                        )
                        process.kill()
                        break
                else:
                    error.extend(chunk)
            if limit_error is not None:
                break
    finally:
        selector.close()
        if process.poll() is None:
            process.kill()
        process.wait()
        process.stdout.close()
        process.stderr.close()
    if limit_error is not None:
        raise RuntimeError(limit_error)
    if process.returncode != 0:
        detail = error.decode("utf-8", errors="replace").strip()
        raise RuntimeError(
            "TypeScript source oracle failed"
            + (f": {detail[:2_000]}" if detail else "")
        )
    return bytes(output)


def _safe_oracle_file(value: object, context: str) -> str:
    if not isinstance(value, str) or not value:
        raise RuntimeError(f"{context} must be a non-empty relative path")
    relative = Path(value.replace("\\", "/"))
    if relative.is_absolute() or ".." in relative.parts:
        raise RuntimeError(f"{context} escapes the source root")
    normalized = relative.as_posix()
    if normalized in {"", "."} or any(part in {"", "."} for part in relative.parts):
        raise RuntimeError(f"{context} is not normalized")
    return normalized


def _oracle_construct(value: object, index: int) -> SourceConstruct:
    if not isinstance(value, dict):
        raise RuntimeError(f"oracle constructs[{index}] must be an object")
    required = {
        "sourceFile",
        "relation",
        "capability",
        "ownerQualifiedName",
        "targetSpelling",
        "qualifier",
        "startByte",
        "endByte",
        "startLine",
    }
    if set(value) != required:
        raise RuntimeError(f"oracle constructs[{index}] has an invalid schema")
    source_file = _safe_oracle_file(value["sourceFile"], f"constructs[{index}].sourceFile")
    text_values = (
        ("relation", value["relation"]),
        ("capability", value["capability"]),
        ("ownerQualifiedName", value["ownerQualifiedName"]),
        ("targetSpelling", value["targetSpelling"]),
    )
    if any(not isinstance(item, str) or not item for _, item in text_values):
        raise RuntimeError(f"oracle constructs[{index}] has invalid text fields")
    qualifier = value["qualifier"]
    if qualifier is not None and (not isinstance(qualifier, str) or not qualifier):
        raise RuntimeError(f"oracle constructs[{index}].qualifier is invalid")
    start_byte = value["startByte"]
    end_byte = value["endByte"]
    start_line = value["startLine"]
    if (
        isinstance(start_byte, bool)
        or not isinstance(start_byte, int)
        or isinstance(end_byte, bool)
        or not isinstance(end_byte, int)
        or isinstance(start_line, bool)
        or not isinstance(start_line, int)
        or start_byte < 0
        or end_byte <= start_byte
        or start_line <= 0
    ):
        raise RuntimeError(f"oracle constructs[{index}] has an invalid source range")
    return SourceConstruct(
        source_file,
        value["relation"],
        value["capability"],
        value["ownerQualifiedName"],
        value["targetSpelling"],
        qualifier,
        start_byte,
        end_byte,
        start_line,
    )


def _oracle_typed_range(value: Mapping[str, object], context: str) -> tuple[int, int, int]:
    start_byte = value.get("startByte")
    end_byte = value.get("endByte")
    start_line = value.get("startLine")
    if (
        isinstance(start_byte, bool)
        or not isinstance(start_byte, int)
        or isinstance(end_byte, bool)
        or not isinstance(end_byte, int)
        or isinstance(start_line, bool)
        or not isinstance(start_line, int)
        or start_byte < 0
        or end_byte <= start_byte
        or start_line <= 0
    ):
        raise RuntimeError(f"{context} has an invalid source range")
    return start_byte, end_byte, start_line


def _oracle_declaration(value: object, index: int) -> Mapping[str, object]:
    context = f"oracle declarations[{index}]"
    if not isinstance(value, dict):
        raise RuntimeError(f"{context} must be an object")
    required = {
        "sourceFile",
        "kind",
        "name",
        "qualifiedName",
        "ownerQualifiedName",
        "namespace",
        "startByte",
        "endByte",
        "startLine",
        "parameterCount",
        "minimumParameterCount",
        "hasRestParameter",
    }
    if set(value) != required:
        raise RuntimeError(f"{context} has an invalid schema")
    _safe_oracle_file(value["sourceFile"], f"{context}.sourceFile")
    for field in ("kind", "name", "qualifiedName", "ownerQualifiedName", "namespace"):
        if not isinstance(value[field], str) or not value[field]:
            raise RuntimeError(f"{context}.{field} is invalid")
    _oracle_typed_range(value, context)
    for field in ("parameterCount", "minimumParameterCount"):
        count = value[field]
        if (
            count is not None
            and (isinstance(count, bool) or not isinstance(count, int) or count < 0)
        ):
            raise RuntimeError(f"{context}.{field} is invalid")
    if value["parameterCount"] is None and value["minimumParameterCount"] is not None:
        raise RuntimeError(f"{context} has a minimum parameter count without a parameter count")
    if (
        value["parameterCount"] is not None
        and value["minimumParameterCount"] is not None
        and value["minimumParameterCount"] > value["parameterCount"]
    ):
        raise RuntimeError(f"{context} has an invalid parameter count relationship")
    if not isinstance(value["hasRestParameter"], bool):
        raise RuntimeError(f"{context}.hasRestParameter is invalid")
    return value


def _oracle_scope(value: object, index: int) -> Mapping[str, object]:
    context = f"oracle scopes[{index}]"
    if not isinstance(value, dict):
        raise RuntimeError(f"{context} must be an object")
    required = {
        "sourceFile",
        "scopeId",
        "kind",
        "ownerQualifiedName",
        "parentScopeId",
        "startByte",
        "endByte",
        "startLine",
    }
    if set(value) != required:
        raise RuntimeError(f"{context} has an invalid schema")
    _safe_oracle_file(value["sourceFile"], f"{context}.sourceFile")
    for field in ("scopeId", "kind", "ownerQualifiedName"):
        if not isinstance(value[field], str) or not value[field]:
            raise RuntimeError(f"{context}.{field} is invalid")
    parent = value["parentScopeId"]
    if parent is not None and (not isinstance(parent, str) or not parent):
        raise RuntimeError(f"{context}.parentScopeId is invalid")
    _oracle_typed_range(value, context)
    return value


def _oracle_call(
    value: object,
    index: int,
    *,
    construction: bool = False,
) -> Mapping[str, object]:
    context = f"oracle {'constructions' if construction else 'calls'}[{index}]"
    if not isinstance(value, dict):
        raise RuntimeError(f"{context} must be an object")
    required = {
        "sourceFile",
        "relation",
        "kind",
        "ownerQualifiedName",
        "targetSpelling",
        "qualifier",
        "targetKind",
        "startByte",
        "endByte",
        "startLine",
        "callStartByte",
        "callEndByte",
        "argumentCount",
        "hasSpreadArgument",
        "optional",
    }
    if set(value) != required:
        raise RuntimeError(f"{context} has an invalid schema")
    expected_relation = "instantiates" if construction else "calls"
    expected_kind = "construction" if construction else "call"
    if value["relation"] != expected_relation or value["kind"] != expected_kind:
        raise RuntimeError(f"{context} has an invalid relation or kind")
    _safe_oracle_file(value["sourceFile"], f"{context}.sourceFile")
    for field in ("relation", "kind", "ownerQualifiedName", "targetSpelling", "targetKind"):
        if not isinstance(value[field], str) or not value[field]:
            raise RuntimeError(f"{context}.{field} is invalid")
    qualifier = value["qualifier"]
    if qualifier is not None and (not isinstance(qualifier, str) or not qualifier):
        raise RuntimeError(f"{context}.qualifier is invalid")
    start_byte, end_byte, _ = _oracle_typed_range(value, context)
    call_start = value["callStartByte"]
    call_end = value["callEndByte"]
    if (
        isinstance(call_start, bool)
        or not isinstance(call_start, int)
        or isinstance(call_end, bool)
        or not isinstance(call_end, int)
        or call_start > start_byte
        or call_end < call_start
        or call_end < end_byte
        or call_end <= call_start
    ):
        raise RuntimeError(f"{context} has an invalid call range")
    argument_count = value["argumentCount"]
    if (
        isinstance(argument_count, bool)
        or not isinstance(argument_count, int)
        or argument_count < 0
    ):
        raise RuntimeError(f"{context}.argumentCount is invalid")
    for field in ("hasSpreadArgument", "optional"):
        if not isinstance(value[field], bool):
            raise RuntimeError(f"{context}.{field} is invalid")
    return value


def _oracle_statement_range(
    value: Mapping[str, object],
    context: str,
    start_byte: int,
    end_byte: int,
) -> None:
    statement_start = value.get("statementStartByte")
    statement_end = value.get("statementEndByte")
    statement_line = value.get("statementStartLine")
    if (
        isinstance(statement_start, bool)
        or not isinstance(statement_start, int)
        or isinstance(statement_end, bool)
        or not isinstance(statement_end, int)
        or isinstance(statement_line, bool)
        or not isinstance(statement_line, int)
        or statement_start < 0
        or statement_end <= statement_start
        or statement_line <= 0
        or statement_start > start_byte
        or statement_end < end_byte
    ):
        raise RuntimeError(f"{context} has an invalid enclosing statement range")


def _oracle_optional_text(value: object, context: str) -> None:
    if value is not None and (not isinstance(value, str) or not value):
        raise RuntimeError(f"{context} is invalid")


def _oracle_construction(value: object, index: int) -> Mapping[str, object]:
    context = f"oracle constructions[{index}]"
    if not isinstance(value, dict):
        raise RuntimeError(f"{context} must be an object")
    required = {
        "sourceFile",
        "relation",
        "kind",
        "ownerQualifiedName",
        "targetSpelling",
        "qualifier",
        "targetKind",
        "startByte",
        "endByte",
        "startLine",
        "callStartByte",
        "callEndByte",
        "argumentCount",
        "hasSpreadArgument",
        "optional",
    }
    if set(value) != required:
        raise RuntimeError(f"{context} has an invalid schema")
    if value["relation"] != "instantiates" or value["kind"] != "construction":
        raise RuntimeError(f"{context} has an invalid relation or kind")
    return _oracle_call(value, index, construction=True)  # type: ignore[arg-type]


def _oracle_import(value: object, index: int) -> Mapping[str, object]:
    context = f"oracle imports[{index}]"
    if not isinstance(value, dict):
        raise RuntimeError(f"{context} must be an object")
    required = {
        "sourceFile",
        "relation",
        "kind",
        "ownerQualifiedName",
        "moduleSpecifier",
        "importedName",
        "localName",
        "isTypeOnly",
        "startByte",
        "endByte",
        "startLine",
        "statementStartByte",
        "statementEndByte",
        "statementStartLine",
    }
    if set(value) != required:
        raise RuntimeError(f"{context} has an invalid schema")
    if value["relation"] != "imports":
        raise RuntimeError(f"{context}.relation is invalid")
    if value["kind"] not in {
        "side_effect",
        "default",
        "named",
        "namespace",
        "import_equals",
        "dynamic",
        "require",
        "import_type",
    }:
        raise RuntimeError(f"{context}.kind is invalid")
    _safe_oracle_file(value["sourceFile"], f"{context}.sourceFile")
    for field in ("ownerQualifiedName", "moduleSpecifier"):
        if not isinstance(value[field], str) or not value[field]:
            raise RuntimeError(f"{context}.{field} is invalid")
    _oracle_optional_text(value["importedName"], f"{context}.importedName")
    _oracle_optional_text(value["localName"], f"{context}.localName")
    if not isinstance(value["isTypeOnly"], bool):
        raise RuntimeError(f"{context}.isTypeOnly is invalid")
    start_byte, end_byte, _ = _oracle_typed_range(value, context)
    _oracle_statement_range(value, context, start_byte, end_byte)
    return value


def _oracle_reexport(value: object, index: int) -> Mapping[str, object]:
    context = f"oracle reexports[{index}]"
    if not isinstance(value, dict):
        raise RuntimeError(f"{context} must be an object")
    required = {
        "sourceFile",
        "relation",
        "kind",
        "ownerQualifiedName",
        "moduleSpecifier",
        "exportedName",
        "localName",
        "isTypeOnly",
        "startByte",
        "endByte",
        "startLine",
        "statementStartByte",
        "statementEndByte",
        "statementStartLine",
    }
    if set(value) != required:
        raise RuntimeError(f"{context} has an invalid schema")
    if value["relation"] != "reexports":
        raise RuntimeError(f"{context}.relation is invalid")
    if value["kind"] not in {"star", "named", "namespace", "local", "default"}:
        raise RuntimeError(f"{context}.kind is invalid")
    _safe_oracle_file(value["sourceFile"], f"{context}.sourceFile")
    if not isinstance(value["ownerQualifiedName"], str) or not value["ownerQualifiedName"]:
        raise RuntimeError(f"{context}.ownerQualifiedName is invalid")
    _oracle_optional_text(value["moduleSpecifier"], f"{context}.moduleSpecifier")
    _oracle_optional_text(value["exportedName"], f"{context}.exportedName")
    _oracle_optional_text(value["localName"], f"{context}.localName")
    if not isinstance(value["isTypeOnly"], bool):
        raise RuntimeError(f"{context}.isTypeOnly is invalid")
    start_byte, end_byte, _ = _oracle_typed_range(value, context)
    _oracle_statement_range(value, context, start_byte, end_byte)
    return value


def _oracle_base(value: object, index: int) -> Mapping[str, object]:
    context = f"oracle bases[{index}]"
    if not isinstance(value, dict):
        raise RuntimeError(f"{context} must be an object")
    required = {
        "sourceFile",
        "relation",
        "kind",
        "ownerQualifiedName",
        "targetSpelling",
        "qualifier",
        "startByte",
        "endByte",
        "startLine",
        "statementStartByte",
        "statementEndByte",
        "statementStartLine",
    }
    if set(value) != required:
        raise RuntimeError(f"{context} has an invalid schema")
    if value["relation"] not in {"extends", "implements"} or value["kind"] != value["relation"]:
        raise RuntimeError(f"{context}.relation or kind is invalid")
    _safe_oracle_file(value["sourceFile"], f"{context}.sourceFile")
    for field in ("ownerQualifiedName", "targetSpelling"):
        if not isinstance(value[field], str) or not value[field]:
            raise RuntimeError(f"{context}.{field} is invalid")
    _oracle_optional_text(value["qualifier"], f"{context}.qualifier")
    start_byte, end_byte, _ = _oracle_typed_range(value, context)
    _oracle_statement_range(value, context, start_byte, end_byte)
    return value


def _oracle_member(value: object, index: int) -> Mapping[str, object]:
    context = f"oracle members[{index}]"
    if not isinstance(value, dict):
        raise RuntimeError(f"{context} must be an object")
    required = {
        "sourceFile",
        "relation",
        "kind",
        "ownerQualifiedName",
        "targetSpelling",
        "qualifier",
        "accessKind",
        "optional",
        "startByte",
        "endByte",
        "startLine",
        "statementStartByte",
        "statementEndByte",
        "statementStartLine",
    }
    if set(value) != required:
        raise RuntimeError(f"{context} has an invalid schema")
    if value["relation"] != "accesses" or value["kind"] not in {"property", "computed_literal"}:
        raise RuntimeError(f"{context}.relation or kind is invalid")
    if value["accessKind"] not in {"read", "write"}:
        raise RuntimeError(f"{context}.accessKind is invalid")
    _safe_oracle_file(value["sourceFile"], f"{context}.sourceFile")
    for field in ("ownerQualifiedName", "targetSpelling"):
        if not isinstance(value[field], str) or not value[field]:
            raise RuntimeError(f"{context}.{field} is invalid")
    _oracle_optional_text(value["qualifier"], f"{context}.qualifier")
    if not isinstance(value["optional"], bool):
        raise RuntimeError(f"{context}.optional is invalid")
    start_byte, end_byte, _ = _oracle_typed_range(value, context)
    _oracle_statement_range(value, context, start_byte, end_byte)
    return value


def _oracle_reference(value: object, index: int) -> Mapping[str, object]:
    context = f"oracle references[{index}]"
    if not isinstance(value, dict):
        raise RuntimeError(f"{context} must be an object")
    required = {
        "sourceFile",
        "relation",
        "kind",
        "ownerQualifiedName",
        "targetSpelling",
        "qualifier",
        "startByte",
        "endByte",
        "startLine",
    }
    if set(value) != required:
        raise RuntimeError(f"{context} has an invalid schema")
    if value["relation"] != "references" or value["kind"] not in {
        "identifier",
        "type",
        "jsx",
        "jsx_value",
        "jsx_spread",
        "jsx_child",
    }:
        raise RuntimeError(f"{context}.relation or kind is invalid")
    _safe_oracle_file(value["sourceFile"], f"{context}.sourceFile")
    for field in ("ownerQualifiedName", "targetSpelling"):
        if not isinstance(value[field], str) or not value[field]:
            raise RuntimeError(f"{context}.{field} is invalid")
    _oracle_optional_text(value["qualifier"], f"{context}.qualifier")
    _oracle_typed_range(value, context)
    return value


def _typescript_inventory_from_payload(
    payload: object,
    root: Path,
) -> SourceConstructInventory:
    root = root.resolve()
    if not isinstance(payload, dict):
        raise RuntimeError("TypeScript source oracle output must be an object")
    required = {
        "schema",
        "provider",
        "metadata",
        "scannedFiles",
        "parsedFiles",
        "rejectedFiles",
        "constructs",
    }
    optional = {
        "projects",
        "diagnostics",
        "scopes",
        "declarations",
        "calls",
        "constructions",
        "imports",
        "reexports",
        "bases",
        "members",
        "references",
    }
    if not required.issubset(payload) or set(payload) - required - optional:
        raise RuntimeError("TypeScript source oracle output has an invalid schema")
    if payload["schema"] != _TYPESCRIPT_ORACLE_SCHEMA:
        raise RuntimeError(f"unsupported TypeScript source oracle schema: {payload['schema']!r}")
    if payload["provider"] != _TYPESCRIPT_ORACLE_PROVIDER:
        raise RuntimeError(
            "TypeScript source oracle provider mismatch: "
            f"expected {_TYPESCRIPT_ORACLE_PROVIDER!r}, observed {payload['provider']!r}"
        )
    metadata = payload["metadata"]
    if not isinstance(metadata, dict) or any(
        not isinstance(key, str)
        or not key
        or not isinstance(value, str)
        or not value
        for key, value in metadata.items()
    ):
        raise RuntimeError("TypeScript source oracle metadata is invalid")
    compiler_version = metadata.get("compilerVersion")
    script_sha256 = metadata.get("scriptSha256")
    if compiler_version != "5.9.3" or not re.fullmatch(r"[0-9a-f]{64}", script_sha256 or ""):
        raise RuntimeError("TypeScript source oracle metadata is not pinned")
    for digest_name in ("configDigest", "sourceDigest"):
        if digest_name in metadata and not re.fullmatch(
            r"[0-9a-f]{64}", metadata[digest_name]
        ):
            raise RuntimeError(
                f"TypeScript source oracle metadata {digest_name} is invalid"
            )
    project_mode = metadata.get("projectMode")
    if project_mode is not None and project_mode not in {"project", "fallback", "tree"}:
        raise RuntimeError("TypeScript source oracle metadata projectMode is invalid")
    diagnostic_count = metadata.get("diagnosticCount")
    if diagnostic_count is not None and not re.fullmatch(r"[0-9]+", diagnostic_count):
        raise RuntimeError("TypeScript source oracle metadata diagnosticCount is invalid")
    scanned = payload["scannedFiles"]
    parsed = payload["parsedFiles"]
    if (
        isinstance(scanned, bool)
        or not isinstance(scanned, int)
        or isinstance(parsed, bool)
        or not isinstance(parsed, int)
        or scanned < 0
        or parsed < 0
        or parsed > scanned
    ):
        raise RuntimeError("TypeScript source oracle coverage counts are invalid")
    rejected = payload["rejectedFiles"]
    if not isinstance(rejected, list):
        raise RuntimeError("TypeScript source oracle rejectedFiles must be an array")
    rejected_files = tuple(
        sorted({_safe_oracle_file(value, "rejectedFiles[]") for value in rejected})
    )
    constructs = payload["constructs"]
    if not isinstance(constructs, list):
        raise RuntimeError("TypeScript source oracle constructs must be an array")
    parsed_constructs = tuple(
        sorted(
            {_oracle_construct(value, index) for index, value in enumerate(constructs)},
            key=_source_construct_key,
        )
    )
    for construct in parsed_constructs:
        path = (root / construct.source_file).resolve()
        try:
            path.relative_to(root)
        except ValueError as error:
            raise RuntimeError(
                f"oracle construct escapes the source root: {construct.source_file}"
            ) from error
        if not path.is_file():
            raise RuntimeError(f"oracle construct source is missing: {construct.source_file}")
        if construct.end_byte > path.stat().st_size:
            raise RuntimeError(
                f"oracle construct range exceeds source: {construct.source_file}"
            )
    typed_records: dict[str, list[Mapping[str, object]]] = {}
    typed_specs = (
        ("scopes", _oracle_scope),
        ("declarations", _oracle_declaration),
        ("calls", _oracle_call),
        ("constructions", _oracle_construction),
        ("imports", _oracle_import),
        ("reexports", _oracle_reexport),
        ("bases", _oracle_base),
        ("members", _oracle_member),
        ("references", _oracle_reference),
    )
    total_typed_facts = 0
    for field, validator in typed_specs:
        values = payload.get(field)
        if values is None:
            continue
        if not isinstance(values, list):
            raise RuntimeError(f"TypeScript source oracle {field} must be an array")
        if len(values) > _TYPESCRIPT_ORACLE_MAX_TYPED_FACTS:
            raise RuntimeError(
                f"TypeScript source oracle {field} exceeds the configured limit"
            )
        total_typed_facts += len(values)
        if total_typed_facts > _TYPESCRIPT_ORACLE_MAX_TYPED_FACTS:
            raise RuntimeError("TypeScript source oracle typed facts exceed the configured limit")
        typed_records[field] = [validator(value, index) for index, value in enumerate(values)]
    scope_ids: set[str] = set()
    scopes_by_id: dict[str, Mapping[str, object]] = {}
    for scope in typed_records.get("scopes", []):
        scope_id = scope["scopeId"]
        if scope_id in scope_ids:
            raise RuntimeError(f"TypeScript source oracle scope {scope_id!r} is duplicated")
        scope_ids.add(scope_id)
        scopes_by_id[scope_id] = scope
    for scope in typed_records.get("scopes", []):
        parent = scope["parentScopeId"]
        if parent is not None:
            if parent not in scope_ids:
                raise RuntimeError(
                    f"TypeScript source oracle scope parent {parent!r} is missing"
                )
            parent_scope = scopes_by_id[parent]
            if parent_scope["sourceFile"] != scope["sourceFile"]:
                raise RuntimeError(
                    f"TypeScript source oracle scope parent {parent!r} crosses source files"
                )
            parent_start, parent_end, _ = _oracle_typed_range(
                parent_scope, f"scope parent {parent!r}"
            )
            start_byte, end_byte, _ = _oracle_typed_range(
                scope, f"oracle scope {scope['scopeId']!r}"
            )
            if parent_start > start_byte or parent_end < end_byte:
                raise RuntimeError(
                    f"TypeScript source oracle scope parent {parent!r} does not enclose child"
                )
    for scope in typed_records.get("scopes", []):
        chain: set[str] = set()
        current = scope["scopeId"]
        while current is not None:
            if current in chain:
                raise RuntimeError(
                    f"TypeScript source oracle scope parent cycle at {current!r}"
                )
            if current not in scopes_by_id:
                raise RuntimeError(
                    f"TypeScript source oracle scope parent {current!r} is missing"
                )
            chain.add(current)
            current = scopes_by_id[current]["parentScopeId"]
    for field, values in typed_records.items():
        for index, record in enumerate(values):
            source_file = record["sourceFile"]
            source_path = (root / source_file).resolve()
            try:
                source_path.relative_to(root)
            except ValueError as error:
                raise RuntimeError(
                    f"oracle {field}[{index}] escapes the source root: {source_file}"
                ) from error
            if not source_path.is_file():
                raise RuntimeError(f"oracle {field}[{index}] source is missing: {source_file}")
            _, end_byte, _ = _oracle_typed_range(record, f"oracle {field}[{index}]")
            if end_byte > source_path.stat().st_size:
                raise RuntimeError(
                    f"oracle {field}[{index}] range exceeds source: {source_file}"
                )
            if field in {"calls", "constructions"} and record["callEndByte"] > source_path.stat().st_size:
                raise RuntimeError(
                    f"oracle {field}[{index}] call range exceeds source: {source_file}"
                )
            if field in {"imports", "reexports", "bases", "members"} and record["statementEndByte"] > source_path.stat().st_size:
                raise RuntimeError(
                    f"oracle {field}[{index}] statement range exceeds source: {source_file}"
                )
    if len(rejected_files) != scanned - parsed:
        raise RuntimeError(
            "TypeScript source oracle coverage does not account for every scanned file"
        )
    projects = payload.get("projects")
    if projects is not None:
        if not isinstance(projects, list):
            raise RuntimeError("TypeScript source oracle projects must be an array")
        for index, project in enumerate(projects):
            if not isinstance(project, dict):
                raise RuntimeError(f"oracle projects[{index}] must be an object")
            if set(project) != {"configFile", "fileCount", "files", "references", "configDigest"}:
                raise RuntimeError(f"oracle projects[{index}] has an invalid schema")
            config_file = _safe_oracle_file(
                project["configFile"], f"projects[{index}].configFile"
            )
            config_path = (root / config_file).resolve()
            try:
                config_path.relative_to(root)
            except ValueError as error:
                raise RuntimeError(
                    f"oracle project config escapes the source root: {config_file}"
                ) from error
            if not config_path.is_file():
                raise RuntimeError(f"oracle project config is missing: {config_file}")
            file_count = project["fileCount"]
            if isinstance(file_count, bool) or not isinstance(file_count, int) or file_count < 0:
                raise RuntimeError(f"oracle projects[{index}].fileCount is invalid")
            if not isinstance(project["files"], list) or len(project["files"]) != file_count:
                raise RuntimeError(f"oracle projects[{index}].files is invalid")
            project_files: set[str] = set()
            for file_index, file_name in enumerate(project["files"]):
                normalized_file = _safe_oracle_file(
                    file_name, f"projects[{index}].files[{file_index}]"
                )
                if normalized_file in project_files:
                    raise RuntimeError(f"oracle projects[{index}].files is not unique")
                project_files.add(normalized_file)
                source_path = (root / normalized_file).resolve()
                try:
                    source_path.relative_to(root)
                except ValueError as error:
                    raise RuntimeError(
                        f"oracle project file escapes the source root: {normalized_file}"
                    ) from error
                if not source_path.is_file():
                    raise RuntimeError(
                        f"oracle project file is missing: {normalized_file}"
                    )
            if not isinstance(project["references"], list):
                raise RuntimeError(f"oracle projects[{index}].references is invalid")
            for reference_index, reference in enumerate(project["references"]):
                normalized_reference = _safe_oracle_file(
                    reference, f"projects[{index}].references[{reference_index}]"
                )
                reference_path = (root / normalized_reference).resolve()
                try:
                    reference_path.relative_to(root)
                except ValueError as error:
                    raise RuntimeError(
                        f"oracle project reference escapes the source root: {normalized_reference}"
                    ) from error
                if not reference_path.is_file():
                    raise RuntimeError(
                        f"oracle project reference is missing: {normalized_reference}"
                    )
            if not re.fullmatch(r"[0-9a-f]{64}", project["configDigest"]):
                raise RuntimeError(f"oracle projects[{index}].configDigest is invalid")
    diagnostics = payload.get("diagnostics")
    if diagnostics is not None:
        if not isinstance(diagnostics, list):
            raise RuntimeError("TypeScript source oracle diagnostics must be an array")
        for index, diagnostic in enumerate(diagnostics):
            if (
                not isinstance(diagnostic, dict)
                or set(diagnostic) != {"file", "message"}
                or not isinstance(diagnostic["message"], str)
                or not diagnostic["message"]
            ):
                raise RuntimeError(f"oracle diagnostics[{index}] has an invalid schema")
            _safe_oracle_file(diagnostic["file"], f"diagnostics[{index}].file")
    return SourceConstructInventory(
        parsed_constructs,
        scanned,
        parsed,
        rejected_files,
        tuple(sorted(metadata.items())),
    )


def _typescript_payload_from_jsonl(raw: bytes) -> dict[str, object]:
    """Reassemble and validate the bounded source-oracle JSONL stream."""

    try:
        lines = raw.decode("utf-8").splitlines()
    except UnicodeDecodeError as error:
        raise RuntimeError(f"invalid TypeScript source oracle JSONL UTF-8: {error}") from error
    if len(lines) < 2:
        raise RuntimeError("TypeScript source oracle JSONL is incomplete")
    records: list[object] = []
    for index, line in enumerate(lines):
        if not line:
            raise RuntimeError(f"TypeScript source oracle JSONL line {index} is empty")
        try:
            record = json.loads(line)
        except json.JSONDecodeError as error:
            raise RuntimeError(
                f"invalid TypeScript source oracle JSONL line {index}: {error}"
            ) from error
        if not isinstance(record, dict):
            raise RuntimeError(f"TypeScript source oracle JSONL line {index} is not an object")
        records.append(record)
    header = records[0]
    footer = records[-1]
    if (
        header.get("recordType") != "header"
        or footer.get("recordType") != "footer"
        or header.get("schema") != _TYPESCRIPT_ORACLE_JSONL_SCHEMA
        or footer.get("schema") != _TYPESCRIPT_ORACLE_JSONL_SCHEMA
        or header.get("provider") != _TYPESCRIPT_ORACLE_PROVIDER
        or footer.get("provider") != _TYPESCRIPT_ORACLE_PROVIDER
    ):
        raise RuntimeError("TypeScript source oracle JSONL header/footer is invalid")
    metadata = header.get("metadata")
    if not isinstance(metadata, dict):
        raise RuntimeError("TypeScript source oracle JSONL metadata is invalid")
    projects: list[dict[str, object]] = []
    diagnostics: list[dict[str, object]] = []
    constructs: list[dict[str, object]] = []
    scopes: list[dict[str, object]] = []
    declarations: list[dict[str, object]] = []
    calls: list[dict[str, object]] = []
    constructions: list[dict[str, object]] = []
    imports: list[dict[str, object]] = []
    reexports: list[dict[str, object]] = []
    bases: list[dict[str, object]] = []
    members: list[dict[str, object]] = []
    references: list[dict[str, object]] = []
    files: list[dict[str, object]] = []
    record_order = {
        "project": 0,
        "file": 1,
        "diagnostic": 2,
        "construct": 3,
        "scope": 4,
        "declaration": 5,
        "call": 6,
        "construction": 7,
        "import": 8,
        "reexport": 9,
        "base": 10,
        "member": 11,
        "reference": 12,
    }
    previous_order = -1
    for index, record in enumerate(records[1:-1], 1):
        record_type = record.get("recordType")
        if not isinstance(record_type, str):
            raise RuntimeError(
                f"TypeScript source oracle JSONL record {index} has an invalid type"
            )
        current_order = record_order.get(record_type)
        if current_order is None:
            raise RuntimeError(
                f"TypeScript source oracle JSONL record {index} has an invalid type"
            )
        if current_order < previous_order:
            raise RuntimeError(
                f"TypeScript source oracle JSONL record {index} is not deterministically ordered"
            )
        previous_order = current_order
        if record_type == "project":
            projects.append({key: value for key, value in record.items() if key != "recordType"})
        elif record_type == "diagnostic":
            diagnostics.append(
                {key: value for key, value in record.items() if key != "recordType"}
            )
        elif record_type == "construct":
            constructs.append(
                {key: value for key, value in record.items() if key != "recordType"}
            )
        elif record_type == "scope":
            scopes.append({key: value for key, value in record.items() if key != "recordType"})
        elif record_type == "declaration":
            declarations.append(
                {key: value for key, value in record.items() if key != "recordType"}
            )
        elif record_type == "call":
            calls.append({key: value for key, value in record.items() if key != "recordType"})
        elif record_type == "construction":
            constructions.append(
                {key: value for key, value in record.items() if key != "recordType"}
            )
        elif record_type == "import":
            imports.append({key: value for key, value in record.items() if key != "recordType"})
        elif record_type == "reexport":
            reexports.append(
                {key: value for key, value in record.items() if key != "recordType"}
            )
        elif record_type == "base":
            bases.append({key: value for key, value in record.items() if key != "recordType"})
        elif record_type == "member":
            members.append({key: value for key, value in record.items() if key != "recordType"})
        elif record_type == "reference":
            references.append(
                {key: value for key, value in record.items() if key != "recordType"}
            )
        elif record_type == "file":
            files.append(record)
    scanned = header.get("scannedFiles")
    parsed = header.get("parsedFiles")
    if not isinstance(scanned, int) or isinstance(scanned, bool) or scanned < 0:
        raise RuntimeError("TypeScript source oracle JSONL scannedFiles is invalid")
    if not isinstance(parsed, int) or isinstance(parsed, bool) or parsed < 0 or parsed > scanned:
        raise RuntimeError("TypeScript source oracle JSONL parsedFiles is invalid")
    if len(files) != scanned:
        raise RuntimeError("TypeScript source oracle JSONL file coverage is incomplete")
    file_names: set[str] = set()
    file_statuses: dict[str, str] = {}
    rejected: list[str] = []
    for index, record in enumerate(files):
        if set(record) != {"recordType", "file", "status"}:
            raise RuntimeError(f"TypeScript source oracle JSONL file {index} has an invalid schema")
        file_name = _safe_oracle_file(record["file"], f"jsonl files[{index}].file")
        if file_name in file_names:
            raise RuntimeError(f"TypeScript source oracle JSONL file {file_name} is duplicated")
        file_names.add(file_name)
        status = record["status"]
        if status not in {"parsed", "rejected"}:
            raise RuntimeError(f"TypeScript source oracle JSONL file {file_name} has invalid status")
        if status == "rejected":
            rejected.append(file_name)
        file_statuses[file_name] = status
    if list(file_statuses) != sorted(file_statuses):
        raise RuntimeError("TypeScript source oracle JSONL files are not deterministically ordered")
    if len(rejected) != scanned - parsed:
        raise RuntimeError("TypeScript source oracle JSONL coverage counts are inconsistent")
    footer_counts = {
        "scannedFiles": scanned,
        "parsedFiles": parsed,
        "projectCount": len(projects),
        "diagnosticCount": len(diagnostics),
        "constructCount": len(constructs),
        "scopeCount": len(scopes),
        "declarationCount": len(declarations),
        "callCount": len(calls),
        "constructionCount": len(constructions),
        "importCount": len(imports),
        "reexportCount": len(reexports),
        "baseCount": len(bases),
        "memberCount": len(members),
        "referenceCount": len(references),
    }
    for key, expected in footer_counts.items():
        if footer.get(key) != expected:
            raise RuntimeError(
                f"TypeScript source oracle JSONL footer count {key} is inconsistent"
            )
    if header.get("projectCount") != len(projects) or header.get("diagnosticCount") != len(diagnostics):
        raise RuntimeError("TypeScript source oracle JSONL header counts are inconsistent")
    if header.get("constructCount") != len(constructs):
        raise RuntimeError("TypeScript source oracle JSONL construct count is inconsistent")
    for field, values, validator in (
        ("scopes", scopes, _oracle_scope),
        ("declarations", declarations, _oracle_declaration),
        ("calls", calls, _oracle_call),
        ("constructions", constructions, _oracle_construction),
        ("imports", imports, _oracle_import),
        ("reexports", reexports, _oracle_reexport),
        ("bases", bases, _oracle_base),
        ("members", members, _oracle_member),
        ("references", references, _oracle_reference),
    ):
        for index, value in enumerate(values):
            validator(value, index)
            source_file = value["sourceFile"]
            if source_file not in file_statuses:
                raise RuntimeError(
                    f"TypeScript source oracle JSONL {field}[{index}] has an unscanned source"
                )
            if file_statuses[source_file] != "parsed":
                raise RuntimeError(
                    f"TypeScript source oracle JSONL {field}[{index}] belongs to a rejected source"
                )
    if header.get("scopeCount") != len(scopes):
        raise RuntimeError("TypeScript source oracle JSONL scope count is inconsistent")
    if header.get("declarationCount") != len(declarations):
        raise RuntimeError("TypeScript source oracle JSONL declaration count is inconsistent")
    if header.get("callCount") != len(calls):
        raise RuntimeError("TypeScript source oracle JSONL call count is inconsistent")
    for key, values in (
        ("constructionCount", constructions),
        ("importCount", imports),
        ("reexportCount", reexports),
        ("baseCount", bases),
        ("memberCount", members),
        ("referenceCount", references),
    ):
        if header.get(key) != len(values):
            raise RuntimeError(f"TypeScript source oracle JSONL {key} is inconsistent")
    typed_total = sum(
        len(values)
        for values in (
            scopes,
            declarations,
            calls,
            constructions,
            imports,
            reexports,
            bases,
            members,
            references,
        )
    )
    if typed_total > _TYPESCRIPT_ORACLE_MAX_TYPED_FACTS:
        raise RuntimeError("TypeScript source oracle JSONL typed facts exceed the configured limit")
    if footer.get("rejectedFiles") != sorted(rejected):
        raise RuntimeError("TypeScript source oracle JSONL rejected file set is inconsistent")
    for digest_name in ("sourceDigest", "configDigest"):
        if footer.get(digest_name) != metadata.get(digest_name):
            raise RuntimeError(
                f"TypeScript source oracle JSONL footer {digest_name} is inconsistent"
            )
    return {
        "schema": _TYPESCRIPT_ORACLE_SCHEMA,
        "provider": _TYPESCRIPT_ORACLE_PROVIDER,
        "metadata": metadata,
        "scannedFiles": scanned,
        "parsedFiles": parsed,
        "rejectedFiles": sorted(rejected),
        "projects": projects,
        "diagnostics": diagnostics,
        "constructs": constructs,
        "scopes": scopes,
        "declarations": declarations,
        "calls": calls,
        "constructions": constructions,
        "imports": imports,
        "reexports": reexports,
        "bases": bases,
        "members": members,
        "references": references,
    }


def _typescript_compiler_inventory(
    root: Path,
    _include_globs: tuple[str, ...] = (),
    _exclude_globs: tuple[str, ...] = (),
) -> SourceConstructInventory:
    try:
        payload = _typescript_payload_from_jsonl(_bounded_node_oracle(root))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise RuntimeError(f"invalid TypeScript source oracle output: {error}") from error
    except RuntimeError as error:
        raise RuntimeError(f"invalid TypeScript source oracle output: {error}") from error
    return _typescript_inventory_from_payload(payload, root)


def _bounded_ruby_oracle(root: Path) -> tuple[bytes, dict[str, object]]:
    """Run the Ripper oracle with explicit duration and output bounds."""

    if not _RUBY_ORACLE_SCRIPT.is_file():
        raise RuntimeError(f"Ruby source oracle is missing: {_RUBY_ORACLE_SCRIPT}")
    with tempfile.TemporaryDirectory(prefix="compass-ruby-source-oracle-") as directory:
        output = Path(directory) / "ruby-source-oracle.json"
        command = (
            "ruby",
            str(_RUBY_ORACLE_SCRIPT),
            "--root",
            str(root),
            "--output",
            str(output),
        )
        try:
            completed = subprocess.run(
                command,
                cwd=_RUBY_ORACLE_SCRIPT.parents[1],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
                timeout=_RUBY_ORACLE_TIMEOUT_SECONDS,
            )
        except subprocess.TimeoutExpired as error:
            raise RuntimeError(
                f"Ruby source oracle exceeded {_RUBY_ORACLE_TIMEOUT_SECONDS:.0f}s"
            ) from error
        if completed.returncode != 0:
            detail = completed.stderr.decode("utf-8", errors="replace").strip()
            raise RuntimeError(
                "Ruby source oracle failed"
                + (f": {detail[:2_000]}" if detail else "")
            )
        try:
            raw = output.read_bytes()
        except OSError as error:
            raise RuntimeError(f"Ruby source oracle did not write output: {error}") from error
    if len(raw) > _RUBY_ORACLE_OUTPUT_BYTES:
        raise RuntimeError(
            "Ruby source oracle output exceeds "
            f"{_RUBY_ORACLE_OUTPUT_BYTES} bytes"
        )
    try:
        payload = json.loads(raw)
    except json.JSONDecodeError as error:
        raise RuntimeError(f"invalid Ruby source oracle JSON: {error}") from error
    if not isinstance(payload, dict) or payload.get("schema") != _RUBY_ORACLE_SCHEMA:
        raise RuntimeError("Ruby source oracle schema is invalid")
    return raw, payload


def _ruby_inventory_from_payload(
    root: Path,
    payload: Mapping[str, object],
) -> SourceConstructInventory:
    files = payload.get("files")
    if not isinstance(files, list):
        raise RuntimeError("Ruby source oracle files must be an array")
    constructs: list[SourceConstruct] = []
    rejected: list[str] = []
    parsed = 0
    relation_capabilities = {
        "aliases": "aliases",
        "calls": "calls",
        "constructs": "construction",
        "extends": "base_types",
        "imports": "imports",
        "uses_trait": "traits",
    }
    for file_index, item in enumerate(files):
        if not isinstance(item, dict):
            raise RuntimeError(f"Ruby oracle files[{file_index}] must be an object")
        relative = item.get("path")
        status = item.get("status")
        if not isinstance(relative, str) or not relative:
            raise RuntimeError(f"Ruby oracle files[{file_index}].path is invalid")
        safe_relative = _safe_oracle_file(relative, f"Ruby oracle files[{file_index}].path")
        if status not in {"ok", "partial"}:
            raise RuntimeError(f"Ruby oracle files[{file_index}].status is invalid")
        source_path = (root / safe_relative).resolve()
        try:
            source_path.relative_to(root)
        except ValueError as error:
            raise RuntimeError(f"Ruby oracle file escapes the source root: {relative}") from error
        if not source_path.is_file():
            raise RuntimeError(f"Ruby oracle file is missing: {relative}")
        if status != "ok":
            rejected.append(safe_relative)
            continue
        parsed += 1
        contents = source_path.read_bytes()
        declarations = item.get("declarations", [])
        if not isinstance(declarations, list):
            raise RuntimeError(f"Ruby oracle {relative}.declarations is invalid")
        for declaration_index, declaration in enumerate(declarations):
            context = f"Ruby oracle {relative}.declarations[{declaration_index}]"
            if not isinstance(declaration, dict):
                raise RuntimeError(f"{context} must be an object")
            kind = declaration.get("kind")
            qualified_name = declaration.get("qualifiedName")
            anchor = declaration.get("anchor")
            if (
                not isinstance(kind, str)
                or kind not in {"class", "module", "method"}
                or not isinstance(qualified_name, str)
                or not qualified_name
                or not isinstance(anchor, dict)
            ):
                raise RuntimeError(f"{context} has invalid identity fields")
            start = anchor.get("startByte")
            end = anchor.get("endByte")
            line = anchor.get("startLine")
            if (
                isinstance(start, bool)
                or not isinstance(start, int)
                or isinstance(end, bool)
                or not isinstance(end, int)
                or isinstance(line, bool)
                or not isinstance(line, int)
                or start < 0
                or end <= start
                or line <= 0
                or end > len(contents)
            ):
                raise RuntimeError(f"{context}.anchor is invalid")
            if "#" in qualified_name:
                owner = qualified_name.rsplit("#", 1)[0]
            elif "." in qualified_name and kind == "method":
                owner = qualified_name.rsplit(".", 1)[0]
            elif "::" in qualified_name:
                owner = qualified_name.rsplit("::", 1)[0]
            else:
                owner = safe_relative
            constructs.append(
                SourceConstruct(
                    safe_relative,
                    "contains",
                    "ownership",
                    owner,
                    qualified_name,
                    kind,
                    start,
                    end,
                    line,
                )
            )
        relations = item.get("relations", [])
        if not isinstance(relations, list):
            raise RuntimeError(f"Ruby oracle {relative}.relations is invalid")
        for relation_index, relation in enumerate(relations):
            context = f"Ruby oracle {relative}.relations[{relation_index}]"
            if not isinstance(relation, dict):
                raise RuntimeError(f"{context} must be an object")
            relation_name = relation.get("relation")
            source = relation.get("source")
            target = relation.get("target")
            anchor = relation.get("anchor")
            if (
                not isinstance(relation_name, str)
                or relation_name not in relation_capabilities
                or not isinstance(source, str)
                or not source
                or not isinstance(target, str)
                or not target
                or not isinstance(anchor, dict)
            ):
                raise RuntimeError(f"{context} has invalid identity fields")
            start = anchor.get("startByte")
            end = anchor.get("endByte")
            line = anchor.get("startLine")
            if (
                isinstance(start, bool)
                or not isinstance(start, int)
                or isinstance(end, bool)
                or not isinstance(end, int)
                or isinstance(line, bool)
                or not isinstance(line, int)
                or start < 0
                or end <= start
                or line <= 0
                or end > len(contents)
            ):
                raise RuntimeError(f"{context}.anchor is invalid")
            # The oracle's anchor is a byte range, not a line approximation.
            if not contents[start:end]:
                raise RuntimeError(f"{context}.anchor is empty")
            normalized_relation = (
                "instantiates" if relation_name == "constructs" else relation_name
            )
            if normalized_relation == "uses_trait":
                normalized_relation = "implements"
            constructs.append(
                SourceConstruct(
                    safe_relative,
                    normalized_relation,
                    relation_capabilities[relation_name],
                    source,
                    target,
                    relation.get("operation")
                    if isinstance(relation.get("operation"), str)
                    else None,
                    start,
                    end,
                    line,
                )
            )
    ruby_version = payload.get("rubyVersion")
    ruby_revision = payload.get("rubyRevision")
    metadata = []
    if isinstance(ruby_version, str) and ruby_version:
        metadata.append(("rubyVersion", ruby_version))
    if isinstance(ruby_revision, str) and ruby_revision:
        metadata.append(("rubyRevision", ruby_revision))
    return SourceConstructInventory(
        tuple(sorted(set(constructs), key=_source_construct_key)),
        len(files),
        parsed,
        tuple(sorted(rejected)),
        tuple(sorted(metadata)),
    )


def _ruby_ripper_inventory(
    root: Path,
    _include_globs: tuple[str, ...] = (),
    _exclude_globs: tuple[str, ...] = (),
) -> SourceConstructInventory:
    _raw, payload = _bounded_ruby_oracle(root)
    return _ruby_inventory_from_payload(root, payload)


def _language_source_oracle_inventory(
    root: Path,
    language: str,
    script_name: str,
    provider: str,
    suffixes: tuple[str, ...],
    include_globs: tuple[str, ...] = (),
    exclude_globs: tuple[str, ...] = (),
) -> SourceConstructInventory:
    """Run one pinned, source-only language oracle for an audit corpus.

    The subprocess is deliberately a qualification boundary: it receives a
    path and writes JSON, but it never receives permission to run repository
    tooling or build scripts.  The oracle output is independently hashed by
    the common audit inventory code below.
    """

    source_root = root.resolve()
    if not source_root.is_dir():
        return SourceConstructInventory((), 0, 0, (str(source_root),))
    # ``occurrences.py`` lives at ``benchmarks/performance/compass``; the
    # qualification helpers are repository-root scripts, three parents up.
    script = Path(__file__).resolve().parents[3] / "scripts" / script_name
    if not script.is_file():
        return SourceConstructInventory((), 0, 0, (script_name,))
    # Keep the provider file alive while the streaming array iterator consumes
    # it below.  The old context-manager form removed the file before the
    # iterator ran; retaining the object until the function returns preserves
    # bounded parsing without materializing the full JSON document.
    cached_output = os.environ.get(f"COMPASS_{language.upper()}_ORACLE_CACHE")
    temporary_directory = tempfile.TemporaryDirectory(prefix=f"compass-{language}-oracle-")
    if temporary_directory:
        directory = temporary_directory.name
        output = Path(cached_output).expanduser().resolve() if cached_output else Path(directory) / "oracle.json"
        try:
            completed = (
                subprocess.CompletedProcess((), 0, "", "")
                if cached_output
                else subprocess.run(
                    [
                        sys.executable,
                        str(script),
                        "--root",
                        str(source_root),
                        "--output",
                        str(output),
                        *sum(([
                            "--include",
                            pattern,
                        ] for pattern in include_globs), []),
                        *sum(([
                            "--exclude",
                            pattern,
                        ] for pattern in exclude_globs), []),
                    ],
                    cwd=source_root,
                    check=False,
                    text=True,
                    capture_output=True,
                    timeout=300,
                )
            )
        except subprocess.TimeoutExpired:
            return SourceConstructInventory((), 0, 0, (f"{language}:oracle-timeout",))
        if completed.returncode or not output.is_file():
            return SourceConstructInventory(
                (),
                0,
                0,
                (completed.stderr.strip() or completed.stdout.strip() or script_name,),
            )
        try:
            document_language = read_top_level_value(output, "language")
            document_provider = read_top_level_value(output, "provider")
            document_toolchain = read_top_level_value(output, "toolchain")
            document_implementation = read_top_level_value(output, "implementation")
            document_parser_available = read_top_level_value(output, "parserAvailable")
            document_inventory = read_top_level_value(output, "inventorySha256")
            document_scanned = read_top_level_value(output, "scannedFiles")
            document_parsed = read_top_level_value(output, "parsedFiles")
        except (OSError, KeyError, ValueError, TypeError):
            return SourceConstructInventory((), 0, 0, (script_name,))
        if document_provider != provider or document_language != language:
            return SourceConstructInventory((), 0, 0, (f"{language}:provider-mismatch",))
        if not isinstance(document_toolchain, str) or not isinstance(document_implementation, str):
            return SourceConstructInventory((), 0, 0, (f"{language}:provider-metadata",))
        try:
            file_items = iter_top_level_array(output, "files")
            scanned = int(document_scanned)
            parsed = int(document_parsed)
        except (KeyError, TypeError, ValueError):
            return SourceConstructInventory((), 0, 0, (script_name,))
    constructs: list[SourceConstruct] = []
    rejected: list[str] = []
    try:
        for item in file_items:
            if not isinstance(item, dict):
                rejected.append("<invalid-file-record>")
                continue
            relative = item.get("path")
            if not isinstance(relative, str) or Path(relative).suffix.casefold() not in suffixes:
                continue
            if item.get("status") != "ok":
                rejected.append(relative)
                continue
            relations = item.get("relations", [])
            if not isinstance(relations, list):
                rejected.append(relative)
                continue
            for relation in relations:
                if not isinstance(relation, dict):
                    rejected.append(relative)
                    continue
                try:
                    start = int(relation["startByte"])
                    end = int(relation["endByte"])
                    line = int(relation["startLine"])
                    name = str(relation["relation"])
                    capability = str(relation["capability"])
                    owner = str(relation["ownerQualifiedName"])
                    target = str(relation["targetSpelling"])
                except (KeyError, TypeError, ValueError):
                    rejected.append(relative)
                    continue
                if start < 0 or end <= start or line < 1 or not name or not capability:
                    rejected.append(relative)
                    continue
                constructs.append(
                    SourceConstruct(
                        relative,
                        name,
                        capability,
                        owner,
                        target,
                        relation.get("qualifier") if isinstance(relation.get("qualifier"), str) else None,
                        start,
                        end,
                        line,
                    )
                )
    except (KeyError, TypeError, ValueError):
        return SourceConstructInventory((), 0, 0, (f"{language}:invalid-json",))
    metadata = (
        ("oracleInventorySha256", str(document_inventory)),
        ("oracleToolchain", document_toolchain),
        ("oracleImplementation", document_implementation),
        ("parserAvailable", str(document_parser_available).lower()),
    )
    result = SourceConstructInventory(
        tuple(sorted(set(constructs), key=_source_construct_key)),
        scanned,
        parsed,
        tuple(sorted(set(rejected))),
        metadata,
    )
    temporary_directory.cleanup()
    return result


def _swift_source_oracle_inventory(
    root: Path,
    include_globs: tuple[str, ...] = (),
    exclude_globs: tuple[str, ...] = (),
) -> SourceConstructInventory:
    return _language_source_oracle_inventory(
        root,
        "swift",
        "swift_source_oracle.py",
        "swift-syntax-source-oracle",
        (".swift",),
        include_globs,
        exclude_globs,
    )


def _dart_source_oracle_inventory(
    root: Path,
    include_globs: tuple[str, ...] = (),
    exclude_globs: tuple[str, ...] = (),
) -> SourceConstructInventory:
    return _language_source_oracle_inventory(
        root,
        "dart",
        "dart_source_oracle.py",
        "dart-analyzer-source-oracle",
        (".dart",),
        include_globs,
        exclude_globs,
    )


def _scala_source_oracle_inventory(
    root: Path,
    include_globs: tuple[str, ...] = (),
    exclude_globs: tuple[str, ...] = (),
) -> SourceConstructInventory:
    return _language_source_oracle_inventory(
        root,
        "scala",
        "scala_source_oracle.py",
        "scala-meta-source-oracle",
        (".scala",),
        include_globs,
        exclude_globs,
    )


def _groovy_source_oracle_inventory(
    root: Path,
    include_globs: tuple[str, ...] = (),
    exclude_globs: tuple[str, ...] = (),
) -> SourceConstructInventory:
    return _language_source_oracle_inventory(
        root,
        "groovy",
        "groovy_source_oracle.py",
        "groovy-compilation-unit-source-oracle",
        (".groovy", ".gradle"),
        include_globs,
        exclude_globs,
    )


def _collector_only_construct_parser(
    _root: Path,
    _path: Path,
) -> tuple[SourceConstruct, ...] | None:
    """Placeholder for providers whose parser runs once per project root."""

    return None


DEFAULT_STATEMENT_PROVIDERS: Mapping[str, StatementProvider] = {
    ".py": _python_statement_spans,
}

DEFAULT_CONSTRUCT_PROVIDERS: Mapping[str, ConstructProvider] = {
    "python": ConstructProvider("python_ast", (".py",), _python_constructs),
    "python-frameworks": ConstructProvider(
        "python_framework_ast_v1",
        (".py",),
        _collector_only_construct_parser,
        _python_framework_inventory,
    ),
    "typescript": ConstructProvider(
        _TYPESCRIPT_ORACLE_PROVIDER,
        (".ts", ".tsx", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs", ".d.ts"),
        _collector_only_construct_parser,
        _typescript_compiler_inventory,
    ),
    "javascript": ConstructProvider(
        _TYPESCRIPT_ORACLE_PROVIDER,
        (".js", ".jsx", ".mjs", ".cjs"),
        _collector_only_construct_parser,
        _typescript_compiler_inventory,
    ),
    "ruby": ConstructProvider(
        _RUBY_ORACLE_PROVIDER,
        (".rb", ".rake"),
        _collector_only_construct_parser,
        _ruby_ripper_inventory,
    ),
    "swift": ConstructProvider(
        "swift-syntax-source-oracle",
        (".swift",),
        _collector_only_construct_parser,
        _swift_source_oracle_inventory,
    ),
    "dart": ConstructProvider(
        "dart-analyzer-source-oracle",
        (".dart",),
        _collector_only_construct_parser,
        _dart_source_oracle_inventory,
    ),
    "scala": ConstructProvider(
        "scala-meta-source-oracle",
        (".scala",),
        _collector_only_construct_parser,
        _scala_source_oracle_inventory,
    ),
    "groovy": ConstructProvider(
        "groovy-compilation-unit-source-oracle",
        (".groovy", ".gradle"),
        _collector_only_construct_parser,
        _groovy_source_oracle_inventory,
    ),
}


def independent_source_provider_identity(language: str) -> str | None:
    provider = DEFAULT_CONSTRUCT_PROVIDERS.get(language.casefold())
    return provider.identity if provider is not None else None


def has_independent_source_provider(
    language: str,
    providers: Mapping[str, ConstructProvider] = DEFAULT_CONSTRUCT_PROVIDERS,
) -> bool:
    return language.casefold() in providers


def independent_source_constructs(
    root: Path,
    language: str,
    providers: Mapping[str, ConstructProvider] = DEFAULT_CONSTRUCT_PROVIDERS,
) -> tuple[SourceConstruct, ...]:
    """Collect independent source candidates without reading the graph."""

    return independent_source_inventory(root, language, providers=providers).constructs


def independent_source_inventory(
    root: Path,
    language: str,
    providers: Mapping[str, ConstructProvider] = DEFAULT_CONSTRUCT_PROVIDERS,
    *,
    include_globs: tuple[str, ...] = (),
    exclude_globs: tuple[str, ...] = (),
) -> SourceConstructInventory:
    """Collect source candidates and explicit parser-coverage evidence."""

    root = root.resolve()
    provider = providers.get(language.casefold())
    if provider is None:
        return SourceConstructInventory((), 0, 0, ())
    if provider.collect is not None:
        return provider.collect(root, include_globs, exclude_globs)
    constructs: list[SourceConstruct] = []
    scanned = 0
    parsed = 0
    rejected: list[str] = []
    paths = {
        path
        for suffix in provider.suffixes
        for path in root.rglob(f"*{suffix}")
    }
    for path in sorted(paths):
        resolved = path.resolve()
        try:
            relative = resolved.relative_to(root).as_posix()
        except ValueError:
            rejected.append(path.relative_to(root).as_posix())
            continue
        if include_globs and not any(
            _qualification_glob_matches(relative, pattern) for pattern in include_globs
        ):
            continue
        if any(
            _qualification_glob_matches(relative, pattern) for pattern in exclude_globs
        ):
            continue
        if resolved.is_file():
            scanned += 1
            extracted = provider.parse(root, resolved)
            if extracted is None:
                rejected.append(relative)
            else:
                parsed += 1
                constructs.extend(extracted)
    return SourceConstructInventory(
        tuple(sorted(set(constructs), key=_source_construct_key)),
        scanned,
        parsed,
        tuple(rejected),
    )


def source_construct_inventory_sha256(
    language: str,
    inventory: SourceConstructInventory,
) -> str:
    payload = {
        "provider": independent_source_provider_identity(language),
        "scannedFiles": inventory.scanned_files,
        "parsedFiles": inventory.parsed_files,
        "rejectedFiles": list(inventory.rejected_files),
        **(
            {"providerMetadata": dict(inventory.provider_metadata)}
            if inventory.provider_metadata
            else {}
        ),
        "constructs": [
            {
                "sourceFile": construct.source_file,
                "relation": construct.relation,
                "capability": construct.capability,
                "ownerQualifiedName": construct.owner_qualified_name,
                "targetSpelling": construct.target_spelling,
                "qualifier": construct.qualifier,
                "startByte": construct.start_byte,
                "endByte": construct.end_byte,
                "startLine": construct.start_line,
                **(
                    {"frameworkPack": construct.framework_pack}
                    if construct.framework_pack is not None
                    else {}
                ),
            }
            for construct in inventory.constructs
        ],
    }
    encoded = json.dumps(
        payload,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
    ).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


class SourceOccurrenceOracle:
    """Proves whether two graph locations belong to one source statement.

    Source paths are resolved beneath the pinned corpus root. Unsupported,
    missing, malformed, symlink-escaped, or unparsable inputs fail closed.
    """

    def __init__(
        self,
        root: Path,
        providers: Mapping[str, StatementProvider] = DEFAULT_STATEMENT_PROVIDERS,
    ) -> None:
        self._root = root.resolve()
        self._providers = dict(providers)
        self._cache: dict[Path, StatementSpans] = {}

    def same_statement(
        self,
        relation: str,
        source_file: str,
        left_location: str,
        right_location: str,
    ) -> bool:
        left = self._line(left_location)
        right = self._line(right_location)
        if left is None or right is None or not source_file:
            return False
        path = self._source_path(source_file)
        if path is None:
            return False
        provider = self._providers.get(path.suffix.casefold())
        if provider is None:
            return False
        spans = self._cache.get(path)
        if spans is None:
            spans = provider(path)
            self._cache[path] = spans
        relation_spans = spans.get(relation, ())
        left_span = self._narrowest_span(relation_spans, left)
        right_span = self._narrowest_span(relation_spans, right)
        return left_span is not None and left_span == right_span

    @staticmethod
    def _line(location: str) -> int | None:
        match = _LOCATION.fullmatch(location)
        return int(match.group(1)) if match is not None else None

    @staticmethod
    def _narrowest_span(
        spans: tuple[tuple[int, int], ...], line: int
    ) -> tuple[int, int] | None:
        matches = (span for span in spans if span[0] <= line <= span[1])
        return min(matches, key=lambda span: (span[1] - span[0], span), default=None)

    def _source_path(self, source_file: str) -> Path | None:
        relative = Path(source_file.replace("\\", "/"))
        if relative.is_absolute():
            return None
        path = (self._root / relative).resolve()
        try:
            path.relative_to(self._root)
        except ValueError:
            return None
        return path if path.is_file() else None
