"""Crash-safe publication helpers for qualification evidence."""

from __future__ import annotations

from contextlib import contextmanager
import os
from pathlib import Path
import tempfile
from typing import BinaryIO, Iterator


def _sync_directory(path: Path) -> None:
    if os.name == "nt":
        return
    directory_descriptor = os.open(path, os.O_RDONLY)
    try:
        os.fsync(directory_descriptor)
    finally:
        os.close(directory_descriptor)


@contextmanager
def atomic_binary_writer(path: Path) -> Iterator[BinaryIO]:
    """Yield a binary stream and publish it atomically only after success."""
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{path.name}.", suffix=".tmp", dir=path.parent
    )
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "wb") as stream:
            yield stream
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
        _sync_directory(path.parent)
    finally:
        if temporary.exists():
            temporary.unlink()


def atomic_write_text(path: Path, content: str) -> None:
    """Publish UTF-8 text atomically after flushing file and directory state."""
    with atomic_binary_writer(path) as stream:
        stream.write(content.encode("utf-8"))
