"""Repository-rooted source statement evidence for graph comparison."""

from __future__ import annotations

import ast
from collections.abc import Callable, Mapping
from dataclasses import dataclass
import hashlib
import json
from pathlib import Path
import re
import tokenize


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
    )


@dataclass(frozen=True)
class SourceConstructInventory:
    constructs: tuple[SourceConstruct, ...]
    scanned_files: int
    parsed_files: int
    rejected_files: tuple[str, ...]


SourceConstructParser = Callable[
    [Path, Path],
    tuple[SourceConstruct, ...] | None,
]


@dataclass(frozen=True)
class ConstructProvider:
    identity: str
    suffixes: tuple[str, ...]
    parse: SourceConstructParser


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


DEFAULT_STATEMENT_PROVIDERS: Mapping[str, StatementProvider] = {
    ".py": _python_statement_spans,
}

DEFAULT_CONSTRUCT_PROVIDERS: Mapping[str, ConstructProvider] = {
    "python": ConstructProvider("python_ast", (".py",), _python_constructs),
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

    return independent_source_inventory(root, language, providers).constructs


def independent_source_inventory(
    root: Path,
    language: str,
    providers: Mapping[str, ConstructProvider] = DEFAULT_CONSTRUCT_PROVIDERS,
) -> SourceConstructInventory:
    """Collect source candidates and explicit parser-coverage evidence."""

    root = root.resolve()
    provider = providers.get(language.casefold())
    if provider is None:
        return SourceConstructInventory((), 0, 0, ())
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
            resolved.relative_to(root)
        except ValueError:
            continue
        if resolved.is_file():
            scanned += 1
            extracted = provider.parse(root, resolved)
            if extracted is None:
                rejected.append(resolved.relative_to(root).as_posix())
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
