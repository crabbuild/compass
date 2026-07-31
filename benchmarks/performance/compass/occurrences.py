"""Repository-rooted source statement evidence for graph comparison."""

from __future__ import annotations

import ast
from collections.abc import Callable, Mapping
from pathlib import Path
import re
import tokenize


StatementSpans = Mapping[str, tuple[tuple[int, int], ...]]
StatementProvider = Callable[[Path], StatementSpans]
_LOCATION = re.compile(r"L([1-9][0-9]*)\Z")


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


DEFAULT_STATEMENT_PROVIDERS: Mapping[str, StatementProvider] = {
    ".py": _python_statement_spans,
}


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
