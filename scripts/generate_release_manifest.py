#!/usr/bin/env python3
"""Generate the bounded, deterministic Compass release manifest."""

from __future__ import annotations

import hashlib
import json
import os
import pathlib
import re
import sys
import tempfile


SCHEMA = "compass.release/1"
MANIFEST_NAME = "compass-release.json"
MAX_ARCHIVE_BYTES = 512 * 1024 * 1024
TARGETS = (
    "aarch64-apple-darwin",
    "aarch64-pc-windows-msvc",
    "aarch64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "x86_64-unknown-linux-gnu",
)
TAG_PATTERN = re.compile(r"compass-v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)")


def sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def read_checksum(path: pathlib.Path, archive_name: str) -> str:
    if path.stat().st_size > 4096:
        raise ValueError(f"checksum file exceeds 4096 bytes: {path}")
    fields = path.read_text(encoding="utf-8").split()
    if len(fields) != 2 or fields[1].lstrip("*") != archive_name:
        raise ValueError(f"checksum file does not name {archive_name}: {path}")
    digest = fields[0].lower()
    if len(digest) != 64 or any(character not in "0123456789abcdef" for character in digest):
        raise ValueError(f"checksum file contains an invalid SHA-256 digest: {path}")
    return digest


def generate(tag: str, dist: pathlib.Path) -> dict[str, object]:
    match = TAG_PATTERN.fullmatch(tag)
    if match is None:
        raise ValueError(f"invalid stable Compass release tag: {tag}")
    version = tag.removeprefix("compass-v")
    artifacts: list[dict[str, object]] = []
    for target in TARGETS:
        archive_name = f"compass-{target}.tar.gz"
        archive = dist / archive_name
        checksum = dist / f"{archive_name}.sha256"
        if not archive.is_file() or not checksum.is_file():
            raise ValueError(f"release is missing archive or checksum for {target}")
        size = archive.stat().st_size
        if size <= 0 or size > MAX_ARCHIVE_BYTES:
            raise ValueError(f"release archive has invalid size for {target}: {size}")
        expected = read_checksum(checksum, archive_name)
        actual = sha256(archive)
        if actual != expected:
            raise ValueError(f"release archive checksum mismatch for {target}")
        artifacts.append(
            {
                "target": target,
                "archive": archive_name,
                "sha256": actual,
                "bytes": size,
            }
        )
    return {
        "schema": SCHEMA,
        "version": version,
        "tag": tag,
        "artifacts": artifacts,
    }


def write_manifest(tag: str, dist: pathlib.Path) -> pathlib.Path:
    manifest = generate(tag, dist)
    destination = dist / MANIFEST_NAME
    temporary_path: pathlib.Path | None = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="w",
            encoding="utf-8",
            dir=dist,
            prefix=f".{MANIFEST_NAME}.",
            delete=False,
        ) as stream:
            temporary_path = pathlib.Path(stream.name)
            json.dump(manifest, stream, indent=2)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary_path, destination)
    finally:
        if temporary_path is not None:
            temporary_path.unlink(missing_ok=True)
    return destination


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: generate_release_manifest.py <compass-vVERSION> <dist-directory>", file=sys.stderr)
        return 2
    dist = pathlib.Path(sys.argv[2])
    if not dist.is_dir():
        print(f"error: release directory does not exist: {dist}", file=sys.stderr)
        return 2
    try:
        destination = write_manifest(sys.argv[1], dist)
    except (OSError, UnicodeError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    print(destination)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
