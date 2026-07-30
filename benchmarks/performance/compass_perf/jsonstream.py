"""Bounded streaming access to arrays in a top-level JSON object."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any, Iterator

_CHUNK_CHARS = 1024 * 1024
_MAX_RECORD_CHARS = 16 * 1024 * 1024
_MAX_BUFFER_CHARS = 32 * 1024 * 1024


class _Reader:
    def __init__(self, path: Path, *, chunk_chars: int):
        self.path = path
        self.stream = path.open("r", encoding="utf-8")
        self.chunk_chars = chunk_chars
        self.buffer = ""
        self.position = 0
        self.eof = False
        self.decoder = json.JSONDecoder()

    def close(self) -> None:
        self.stream.close()

    def _compact(self) -> None:
        if self.position >= self.chunk_chars:
            self.buffer = self.buffer[self.position :]
            self.position = 0

    def _fill(self) -> bool:
        if self.eof:
            return False
        self._compact()
        chunk = self.stream.read(self.chunk_chars)
        if chunk:
            self.buffer += chunk
            if len(self.buffer) - self.position > _MAX_BUFFER_CHARS:
                raise ValueError(f"JSON rolling buffer exceeds limit in {self.path}")
            return True
        self.eof = True
        return False

    def peek(self) -> str:
        while self.position >= len(self.buffer):
            if not self._fill():
                return ""
        return self.buffer[self.position]

    def take(self) -> str:
        value = self.peek()
        if value:
            self.position += 1
        return value

    def whitespace(self) -> None:
        while self.peek() and self.peek().isspace():
            self.position += 1

    def expect(self, expected: str) -> None:
        self.whitespace()
        actual = self.take()
        if actual != expected:
            raise ValueError(
                f"expected {expected!r}, found {actual or 'end of file'!r} in {self.path}"
            )

    def decode(self, *, max_chars: int = _MAX_RECORD_CHARS) -> Any:
        self.whitespace()
        start = self.position
        while True:
            try:
                value, end = self.decoder.raw_decode(self.buffer, self.position)
            except json.JSONDecodeError as error:
                if self.eof:
                    raise ValueError(f"invalid or truncated JSON in {self.path}: {error}") from error
                if len(self.buffer) - start > max_chars:
                    raise ValueError(f"JSON record exceeds limit in {self.path}") from error
                self._fill()
                start = min(start, self.position)
                continue
            if end - self.position > max_chars:
                raise ValueError(f"JSON record exceeds limit in {self.path}")
            self.position = end
            return value

    def skip_value(self) -> None:
        self.whitespace()
        first = self.peek()
        if first == "":
            raise ValueError(f"missing JSON value in {self.path}")
        if first == '"':
            self._skip_string()
            return
        if first in "[{":
            self._skip_container()
            return
        consumed = 0
        while self.peek() not in {"", ",", "]", "}"}:
            self.position += 1
            consumed += 1
            if consumed > _MAX_RECORD_CHARS:
                raise ValueError(f"JSON scalar exceeds limit in {self.path}")
        if consumed == 0:
            raise ValueError(f"invalid JSON scalar in {self.path}")

    def _skip_string(self) -> None:
        if self.take() != '"':
            raise ValueError(f"expected JSON string in {self.path}")
        escaped = False
        consumed = 1
        while True:
            char = self.take()
            if not char:
                raise ValueError(f"truncated JSON string in {self.path}")
            consumed += 1
            if consumed > _MAX_RECORD_CHARS:
                raise ValueError(f"JSON string exceeds limit in {self.path}")
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == '"':
                return

    def _skip_container(self) -> None:
        depth = 0
        in_string = False
        escaped = False
        while True:
            char = self.take()
            if not char:
                raise ValueError(f"truncated JSON container in {self.path}")
            if in_string:
                if escaped:
                    escaped = False
                elif char == "\\":
                    escaped = True
                elif char == '"':
                    in_string = False
                continue
            if char == '"':
                in_string = True
            elif char in "[{":
                depth += 1
            elif char in "]}":
                depth -= 1
                if depth == 0:
                    return


def _array(reader: _Reader) -> Iterator[dict[str, object]]:
    reader.expect("[")
    reader.whitespace()
    if reader.peek() == "]":
        reader.take()
        return
    while True:
        value = reader.decode()
        if not isinstance(value, dict):
            raise ValueError(f"top-level graph array contains a non-object in {reader.path}")
        yield value
        reader.whitespace()
        separator = reader.take()
        if separator == "]":
            return
        if separator == "":
            raise ValueError(f"truncated JSON array in {reader.path}")
        if separator != ",":
            raise ValueError(f"expected array separator in {reader.path}")


def iter_top_level_array(
    path: Path,
    key: str,
    *,
    chunk_chars: int = _CHUNK_CHARS,
) -> Iterator[dict[str, object]]:
    reader = _Reader(path, chunk_chars=chunk_chars)
    found = False
    try:
        reader.expect("{")
        first = True
        while True:
            reader.whitespace()
            if reader.peek() == "}":
                reader.take()
                break
            if not first:
                reader.expect(",")
            name = reader.decode()
            if not isinstance(name, str):
                raise ValueError(f"top-level JSON key is not a string in {path}")
            reader.expect(":")
            if name == key:
                if found:
                    raise ValueError(f"duplicate top-level key {key!r} in {path}")
                found = True
                yield from _array(reader)
            else:
                reader.skip_value()
            first = False
        if not found:
            raise KeyError(key)
        reader.whitespace()
        if reader.peek():
            raise ValueError(f"trailing content after top-level object in {path}")
    finally:
        reader.close()


def read_top_level_value(path: Path, key: str) -> Any:
    reader = _Reader(path, chunk_chars=_CHUNK_CHARS)
    try:
        reader.expect("{")
        first = True
        while True:
            reader.whitespace()
            if reader.peek() == "}":
                raise KeyError(key)
            if not first:
                reader.expect(",")
            name = reader.decode()
            reader.expect(":")
            if name == key:
                return reader.decode()
            reader.skip_value()
            first = False
    finally:
        reader.close()


def read_top_level_object_value(path: Path, object_key: str, key: str) -> Any:
    """Read one bounded member without decoding its potentially large parent object."""
    reader = _Reader(path, chunk_chars=_CHUNK_CHARS)
    try:
        reader.expect("{")
        first = True
        while True:
            reader.whitespace()
            if reader.peek() == "}":
                raise KeyError(object_key)
            if not first:
                reader.expect(",")
            name = reader.decode()
            reader.expect(":")
            if name != object_key:
                reader.skip_value()
                first = False
                continue

            reader.expect("{")
            member_first = True
            while True:
                reader.whitespace()
                if reader.peek() == "}":
                    raise KeyError(key)
                if not member_first:
                    reader.expect(",")
                member_name = reader.decode()
                reader.expect(":")
                if member_name == key:
                    return reader.decode()
                reader.skip_value()
                member_first = False
    finally:
        reader.close()
