#!/usr/bin/env python3
"""Small, deterministic source-only qualification oracles.

The qualification helpers intentionally live outside the Compass crates.  They
read checked-in source and manifests only; they never import Compass, invoke a
project build, or execute repository code.  The scanner is deliberately
conservative: it records source constructs with exact UTF-8 byte ranges and
keeps malformed/oversized files explicit instead of silently treating them as
empty inputs.
"""

from __future__ import annotations

import argparse
from bisect import bisect_right
import fnmatch
import hashlib
import json
import os
import re
from pathlib import Path
import subprocess
import tempfile
from typing import Any, Iterable


MAX_FILES = 50_000
MAX_FILE_BYTES = 64 * 1024 * 1024
MAX_TOTAL_BYTES = 1024 * 1024 * 1024
SKIP_DIRECTORIES = frozenset(
    {
        ".git",
        ".dart_tool",
        ".gradle",
        ".idea",
        ".build",
        "build",
        "target",
        "node_modules",
        "vendor",
        "DerivedData",
        "coverage",
    }
)
IDENTIFIER = r"[A-Za-z_][A-Za-z0-9_]*"


class OracleError(RuntimeError):
    """A bounded source-oracle failure."""


PROVIDER_DEFAULTS = {
    "swift": Path(
        "/Volumes/Workspace/crabbuild-target/compass-main/providers/bin/compass-swift-oracle"
    ),
    "dart": Path(
        "/Volumes/Workspace/crabbuild-target/compass-main/providers/bin/compass-dart-oracle"
    ),
    "scala": Path(
        "/Volumes/Workspace/crabbuild-target/compass-main/providers/bin/compass-scala-oracle"
    ),
    "groovy": Path(
        "/Volumes/Workspace/crabbuild-target/compass-main/providers/bin/compass-groovy-oracle"
    ),
}


def _provider_command(language: str) -> Path | None:
    env_name = f"COMPASS_{language.upper()}_ORACLE"
    configured = os.environ.get(env_name)
    if configured:
        return Path(configured).expanduser()
    default = PROVIDER_DEFAULTS.get(language)
    return default if default is not None and default.is_file() else None


def canonical_bytes(value: Any) -> bytes:
    return (
        json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True)
        + "\n"
    ).encode("utf-8")


def digest(value: Any) -> str:
    return hashlib.sha256(canonical_bytes(value).rstrip(b"\n")).hexdigest()


def matches_glob(relative_name: str, pattern: str) -> bool:
    """Match a portable glob, including ``**/`` matching zero directories."""

    if fnmatch.fnmatchcase(relative_name, pattern):
        return True
    # Python's fnmatch treats the slash in ``**/`` literally.  Qualification
    # manifests use pathlib-style recursive globs, where that segment may be
    # empty for a file directly below the named directory.
    return "**/" in pattern and fnmatch.fnmatchcase(
        relative_name,
        pattern.replace("**/", ""),
    )


def _relative_source_files(
    root: Path,
    suffixes: tuple[str, ...],
    include_globs: tuple[str, ...] = (),
    exclude_globs: tuple[str, ...] = (),
) -> list[Path]:
    root = root.resolve()
    if not root.is_dir():
        raise OracleError(f"source root does not exist: {root}")
    files: list[Path] = []
    for path in root.rglob("*"):
        if path.is_symlink() or not path.is_file():
            continue
        try:
            relative = path.relative_to(root)
        except ValueError as error:
            raise OracleError(f"source path escaped root: {path}") from error
        if SKIP_DIRECTORIES.intersection(relative.parts):
            continue
        if path.suffix.casefold() in suffixes:
            relative_name = relative.as_posix()
            if include_globs and not any(matches_glob(relative_name, pattern) for pattern in include_globs):
                continue
            if any(matches_glob(relative_name, pattern) for pattern in exclude_globs):
                continue
            files.append(path)
    files.sort(key=lambda item: item.relative_to(root).as_posix())
    if len(files) > MAX_FILES:
        raise OracleError(f"source file limit exceeded: {len(files)} > {MAX_FILES}")
    return files


def _mask_non_code(
    source: str,
    *,
    hash_comments: bool,
    raw_strings: bool = False,
) -> str:
    """Mask comments and quoted literals while preserving string length/newlines."""

    characters = list(source)
    index = 0
    state = "code"
    quote = ""
    triple = False
    raw_hashes = 0
    raw_triple = False
    while index < len(source):
        current = source[index]
        following = source[index + 1] if index + 1 < len(source) else ""
        if state == "code":
            if current == "/" and following == "/":
                characters[index] = characters[index + 1] = " "
                index += 2
                state = "line_comment"
                continue
            if current == "/" and following == "*":
                characters[index] = characters[index + 1] = " "
                index += 2
                state = "block_comment"
                continue
            if hash_comments and current == "#":
                characters[index] = " "
                index += 1
                state = "line_comment"
                continue
            if raw_strings and current in {"r", "R"} and following in {'"', "'"}:
                marker = (
                    following * 3
                    if source[index + 1 : index + 4] == following * 3
                    else following
                )
                width = 1 + len(marker)
                for position in range(index, min(index + width, len(source))):
                    if characters[position] != "\n":
                        characters[position] = " "
                index += width
                quote = following
                raw_hashes = 0
                raw_triple = len(marker) == 3
                state = "raw_string"
                continue
            if raw_strings and current == "#":
                hashes = 0
                while index + hashes < len(source) and source[index + hashes] == "#":
                    hashes += 1
                marker = '"""' if source[index + hashes : index + hashes + 3] == '"""' else '"'
                if hashes and source[index + hashes : index + hashes + len(marker)] == marker:
                    width = hashes + len(marker)
                    for position in range(index, min(index + width, len(source))):
                        if characters[position] != "\n":
                            characters[position] = " "
                    index += width
                    raw_hashes = hashes
                    raw_triple = marker == '"""'
                    state = "raw_string"
                    continue
            if current in {"\"", "'"}:
                quote = current
                triple = source[index : index + 3] == current * 3
                width = 3 if triple else 1
                for position in range(index, min(index + width, len(source))):
                    if characters[position] != "\n":
                        characters[position] = " "
                index += width
                state = "string"
                continue
            index += 1
            continue
        if state == "line_comment":
            if current == "\n":
                state = "code"
            elif current != "\r":
                characters[index] = " "
            index += 1
            continue
        if state == "block_comment":
            if current == "*" and following == "/":
                characters[index] = characters[index + 1] = " "
                index += 2
                state = "code"
            else:
                if current != "\n" and current != "\r":
                    characters[index] = " "
                index += 1
            continue
        if state == "raw_string":
            marker = quote * 3 if raw_triple else quote
            closing = marker + ("#" * raw_hashes)
            if source.startswith(closing, index):
                for position in range(index, min(index + len(closing), len(source))):
                    if characters[position] != "\n":
                        characters[position] = " "
                index += len(closing)
                state = "code"
                raw_hashes = 0
                raw_triple = False
            else:
                if current != "\n" and current != "\r":
                    characters[index] = " "
                index += 1
            continue
        # string
        if triple and source[index : index + 3] == quote * 3:
            for position in range(index, min(index + 3, len(source))):
                if characters[position] != "\n":
                    characters[position] = " "
            index += 3
            state = "code"
            triple = False
            continue
        if not triple and current == "\\":
            if current != "\n":
                characters[index] = " "
            if index + 1 < len(source):
                if source[index + 1] != "\n":
                    characters[index + 1] = " "
                index += 2
            else:
                index += 1
            continue
        if not triple and current == quote:
            characters[index] = " "
            index += 1
            state = "code"
            continue
        if current != "\n" and current != "\r":
            characters[index] = " "
        index += 1
    return "".join(characters)


def _line_offsets(source: str) -> list[int]:
    offsets = [0]
    total = 0
    for line in source.splitlines(keepends=True):
        total += len(line.encode("utf-8"))
        offsets.append(total)
    if not offsets or offsets[-1] != len(source.encode("utf-8")):
        offsets.append(len(source.encode("utf-8")))
    return offsets


def _line_starts(source: str) -> list[int]:
    starts = [0]
    cursor = source.find("\n")
    while cursor >= 0:
        starts.append(cursor + 1)
        cursor = source.find("\n", cursor + 1)
    return starts


def _byte_range(
    source: str,
    offsets: list[int],
    line_starts: list[int],
    start: int,
    end: int,
) -> tuple[int, int, int]:
    line_index = bisect_right(line_starts, start) - 1
    line = line_index + 1
    line_start = line_starts[line_index]
    start_byte = offsets[line - 1] + len(source[line_start:start].encode("utf-8"))
    end_line_index = bisect_right(line_starts, end) - 1
    end_line_start = line_starts[end_line_index]
    end_byte = offsets[end_line_index] + len(
        source[end_line_start:end].encode("utf-8")
    )
    return start_byte, end_byte, line


def _owner_at(
    declarations: list[dict[str, Any]],
    starts: list[int],
    position: int,
    fallback: str,
) -> str:
    if not declarations:
        return fallback
    index = bisect_right(starts, position) - 1
    if index < 0:
        return fallback
    return str(declarations[index]["qualifiedName"])


def _qualified_name(package: str, name: str, owner: str | None = None) -> str:
    pieces = [piece for piece in (package, owner, name) if piece]
    return "::".join(pieces)


def _declaration_patterns(language: str) -> tuple[re.Pattern[str], set[str]]:
    if language == "swift":
        keywords = "class|struct|enum|actor|protocol|func|init|deinit|typealias|extension"
    elif language == "dart":
        keywords = "class|mixin|extension|enum|typedef|abstract|void|factory|operator"
    elif language == "scala":
        keywords = "class|trait|object|enum|def|val|var|type|given|extension"
    else:
        keywords = "class|interface|trait|enum|record|def|void|static|abstract"
    pattern = re.compile(
        rf"(?P<keyword>\b(?:{keywords})\b)\s+(?P<name>{IDENTIFIER})",
        re.MULTILINE,
    )
    return pattern, set(keywords.split("|"))


def _scan_file(
    root: Path,
    path: Path,
    language: str,
    suffixes: tuple[str, ...],
) -> dict[str, Any]:
    relative = path.relative_to(root).as_posix()
    raw = path.read_bytes()
    if len(raw) > MAX_FILE_BYTES:
        return {"path": relative, "status": "partial", "bytes": len(raw), "declarations": [], "relations": []}
    try:
        source = raw.decode("utf-8")
    except UnicodeDecodeError:
        return {"path": relative, "status": "partial", "bytes": len(raw), "declarations": [], "relations": []}
    offsets = _line_offsets(source)
    line_starts = _line_starts(source)
    masked = _mask_non_code(
        source,
        hash_comments=language in {"scala", "groovy"},
        raw_strings=language in {"swift", "dart"},
    )
    # Keep malformed syntax explicit without treating nested interpolation
    # strings (which can contain their own quotes/braces) as parser failures.
    # The independent compiler-backed providers used for promotion replace
    # this conservative sentinel with their real diagnostic stream.
    if re.search(r"\(\s*=(?![=~])", masked):
        return {
            "path": relative,
            "status": "partial",
            "bytes": len(raw),
            "declarations": [],
            "relations": [],
        }
    package_match = re.search(
        r"\b(?:package|module|namespace)\s+([A-Za-z_][A-Za-z0-9_./:]*)",
        masked,
    )
    package = package_match.group(1).replace(".", "::") if package_match else ""
    declarations: list[dict[str, Any]] = []
    declaration_pattern, declaration_keywords = _declaration_patterns(language)
    for match in declaration_pattern.finditer(masked):
        keyword = match.group("keyword")
        name = match.group("name")
        owner = declarations[-1]["qualifiedName"] if declarations else package
        if owner == package:
            owner = package or None
        qualified = _qualified_name(package, name, owner if owner and owner != package else None)
        start, end, line = _byte_range(
            source,
            offsets,
            line_starts,
            match.start("name"),
            match.end("name"),
        )
        item = {
            "name": name,
            "kind": keyword,
            "qualifiedName": qualified or name,
            "start": match.start(),
            "end": len(masked),
            "startByte": start,
            "endByte": end,
            "startLine": line,
        }
        declarations.append(item)

    relations: list[dict[str, Any]] = []
    declaration_starts = [int(item["start"]) for item in declarations]
    import_pattern = re.compile(
        r"\b(?:import|export|part|use)\s+([^;\n{}]+)", re.MULTILINE
    )
    for match in import_pattern.finditer(masked):
        words = match.group(1).strip().split()
        if not words:
            continue
        target = words[0].strip("'\"")
        if not target:
            continue
        start, end, line = _byte_range(
            source,
            offsets,
            line_starts,
            match.start(1),
            match.start(1) + len(target),
        )
        relation = "reexports" if match.group(0).lstrip().startswith("export") else "imports"
        relations.append(
            {
                "relation": relation,
                "capability": "imports",
                "ownerQualifiedName": package or relative,
                "targetSpelling": target,
                "qualifier": None,
                "startByte": start,
                "endByte": end,
                "startLine": line,
            }
        )

    call_pattern = re.compile(
        rf"(?P<callee>{IDENTIFIER}(?:(?:\.|::|#){IDENTIFIER})*)\s*\(",
        re.MULTILINE,
    )
    ignored = declaration_keywords | {
        "if",
        "for",
        "while",
        "switch",
        "catch",
        "guard",
        "return",
        "where",
        "sizeof",
        "when",
    }
    for match in call_pattern.finditer(masked):
        callee = match.group("callee")
        terminal = re.split(r"[.:#]", callee)[-1]
        if terminal in ignored:
            continue
        # A declaration's name followed by its parameter list is not a call.
        prefix = masked[max(0, match.start("callee") - 24) : match.start("callee")]
        if re.search(r"\b(?:func|def|fun|class|struct|enum|trait|object|interface|extension)\s*$", prefix):
            continue
        parts = re.split(r"[.:#]", callee)
        qualifier = "::".join(parts[:-1]) or None
        start, end, line = _byte_range(
            source,
            offsets,
            line_starts,
            match.start("callee"),
            match.end("callee"),
        )
        relations.append(
            {
                "relation": "calls",
                "capability": "calls",
                "ownerQualifiedName": _owner_at(
                    declarations, declaration_starts, match.start(), package or relative
                ),
                "targetSpelling": terminal,
                "qualifier": qualifier,
                "startByte": start,
                "endByte": end,
                "startLine": line,
            }
        )

    base_pattern = re.compile(
        rf"\b(?:class|struct|enum|actor|trait|object|interface|extension)\s+(?P<name>{IDENTIFIER})\s*:\s*(?P<base>{IDENTIFIER}(?:(?:\.|::){IDENTIFIER})*)",
        re.MULTILINE,
    )
    for match in base_pattern.finditer(masked):
        start, end, line = _byte_range(
            source,
            offsets,
            line_starts,
            match.start("base"),
            match.end("base"),
        )
        owner = _qualified_name(package, match.group("name")) or match.group("name")
        relations.append(
            {
                "relation": "extends",
                "capability": "inheritance",
                "ownerQualifiedName": owner,
                "targetSpelling": match.group("base"),
                "qualifier": None,
                "startByte": start,
                "endByte": end,
                "startLine": line,
            }
        )

    declarations_json = [
        {
            "kind": item["kind"],
            "qualifiedName": item["qualifiedName"],
            "startByte": item["startByte"],
            "endByte": item["endByte"],
            "startLine": item["startLine"],
        }
        for item in declarations
    ]
    declarations_json.sort(key=lambda item: (item["startByte"], item["qualifiedName"], item["kind"]))
    relations.sort(
        key=lambda item: (
            item["startByte"],
            item["endByte"],
            item["relation"],
            item["ownerQualifiedName"],
            item["targetSpelling"],
        )
    )
    return {
        "path": relative,
        "status": "ok",
        "bytes": len(raw),
        "declarations": declarations_json,
        "relations": relations,
    }


def run_oracle(
    root: Path,
    *,
    language: str,
    provider: str,
    toolchain: str,
    implementation: str = "bounded_lexical_scanner",
    parser_available: bool = False,
    suffixes: tuple[str, ...],
    include_globs: tuple[str, ...] = (),
    exclude_globs: tuple[str, ...] = (),
) -> dict[str, Any]:
    root = root.resolve()
    paths = _relative_source_files(root, suffixes, include_globs, exclude_globs)
    files: list[dict[str, Any]] = []
    total_bytes = 0
    for path in paths:
        total_bytes += path.stat().st_size
        if total_bytes > MAX_TOTAL_BYTES:
            raise OracleError(f"source byte limit exceeded: {total_bytes} > {MAX_TOTAL_BYTES}")
        files.append(_scan_file(root, path, language, suffixes))
    inventory = {
        "language": language,
        "provider": provider,
        "toolchain": toolchain,
        "rootRelativeFiles": [item["path"] for item in files],
        "files": files,
    }
    inventory_sha = digest(inventory)
    partial = sum(item["status"] != "ok" for item in files)
    return {
        "schema": f"compass.{language}-source-oracle/1",
        "language": language,
        "provider": provider,
        "toolchain": toolchain,
        "implementation": implementation,
        "parserAvailable": parser_available,
        "limits": {
            "maxFiles": MAX_FILES,
            "maxFileBytes": MAX_FILE_BYTES,
            "maxTotalBytes": MAX_TOTAL_BYTES,
        },
        "scannedFiles": len(files),
        "parsedFiles": len(files) - partial,
        "partialFiles": partial,
        "inventorySha256": inventory_sha,
        "files": files,
    }


def _validate_provider_relation(
    root: Path,
    relative: str,
    relation: Any,
) -> dict[str, Any]:
    if not isinstance(relation, dict):
        raise OracleError(f"parser provider relation in {relative} is not an object")
    required = ("relation", "capability", "ownerQualifiedName", "targetSpelling")
    if any(not isinstance(relation.get(field), str) or not relation[field].strip() for field in required):
        raise OracleError(f"parser provider relation in {relative} has incomplete identity")
    start = relation.get("startByte")
    end = relation.get("endByte")
    line = relation.get("startLine")
    if (
        isinstance(start, bool)
        or not isinstance(start, int)
        or isinstance(end, bool)
        or not isinstance(end, int)
        or isinstance(line, bool)
        or not isinstance(line, int)
        or start < 0
        or end <= start
        or line < 1
    ):
        raise OracleError(f"parser provider relation in {relative} has an invalid range")
    source_path = root / relative
    try:
        source = source_path.read_bytes()
    except OSError as error:
        raise OracleError(f"parser provider source is unavailable: {relative}: {error}") from error
    if end > len(source) or not source[start:end]:
        raise OracleError(f"parser provider relation in {relative} is outside its source")
    qualifier = relation.get("qualifier")
    if qualifier is not None and not isinstance(qualifier, str):
        raise OracleError(f"parser provider qualifier in {relative} is not a string")
    return {
        "relation": relation["relation"],
        "capability": relation["capability"],
        "ownerQualifiedName": relation["ownerQualifiedName"],
        "targetSpelling": relation["targetSpelling"],
        "qualifier": qualifier,
        "startByte": start,
        "endByte": end,
        "startLine": line,
    }


def _run_parser_provider(
    root: Path,
    *,
    language: str,
    provider: str,
    suffixes: tuple[str, ...],
    include_globs: tuple[str, ...],
    exclude_globs: tuple[str, ...],
    command: Path,
) -> dict[str, Any]:
    """Run a pinned parser helper over an explicitly enumerated file set.

    The helper receives a newline-delimited, already validated inventory. It
    therefore cannot broaden the corpus by walking ignored directories or
    following symlinks, and the Python boundary remains responsible for the
    canonical file and digest contract.
    """

    paths = _relative_source_files(root, suffixes, include_globs, exclude_globs)
    relative_paths = [path.relative_to(root.resolve()).as_posix() for path in paths]
    with tempfile.TemporaryDirectory(prefix=f"compass-{language}-parser-provider-") as directory:
        directory_path = Path(directory)
        file_list = directory_path / "files.txt"
        output = directory_path / "provider.json"
        file_list.write_text("\n".join(relative_paths) + ("\n" if relative_paths else ""), encoding="utf-8")
        completed = subprocess.run(
            [
                str(command),
                "--root",
                str(root.resolve()),
                "--files",
                str(file_list),
                "--output",
                str(output),
            ],
            cwd=Path(__file__).resolve().parents[1],
            check=False,
            text=True,
            capture_output=True,
            timeout=900,
        )
        if completed.returncode:
            raise OracleError(
                f"{language} parser provider failed: "
                f"{completed.stderr.strip() or completed.stdout.strip()}"
            )
        try:
            document = json.loads(output.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            raise OracleError(f"{language} parser provider emitted invalid JSON: {error}") from error
    if not isinstance(document, dict):
        raise OracleError(f"{language} parser provider output is not an object")
    if document.get("language") != language or document.get("provider") != provider:
        raise OracleError(f"{language} parser provider identity mismatch")
    for field in ("toolchain", "implementation"):
        if not isinstance(document.get(field), str) or not document[field].strip():
            raise OracleError(f"{language} parser provider omitted {field}")
    if document.get("parserAvailable") is not True:
        raise OracleError(f"{language} parser provider did not assert parserAvailable=true")
    files = document.get("files")
    if not isinstance(files, list):
        raise OracleError(f"{language} parser provider files inventory is not a list")
    observed_paths: list[str] = []
    normalized_files: list[dict[str, Any]] = []
    for item in files:
        if not isinstance(item, dict):
            raise OracleError(f"{language} parser provider emitted a non-object file")
        relative = item.get("path")
        if (
            not isinstance(relative, str)
            or not relative
            or Path(relative).is_absolute()
            or "\\" in relative
            or relative == "."
            or relative.startswith("../")
            or "/../" in f"/{relative}"
        ):
            raise OracleError(f"{language} parser provider emitted an unsafe path")
        if relative not in relative_paths:
            raise OracleError(f"{language} parser provider emitted an unrequested path: {relative}")
        status = item.get("status")
        if status not in {"ok", "partial"}:
            raise OracleError(f"{language} parser provider emitted an invalid status for {relative}")
        relations = item.get("relations", [])
        if not isinstance(relations, list):
            raise OracleError(f"{language} parser provider relations are not a list for {relative}")
        normalized_relations = [
            _validate_provider_relation(root, relative, relation) for relation in relations
        ]
        normalized_relations.sort(
            key=lambda relation: (
                relation["startByte"],
                relation["endByte"],
                relation["relation"],
                relation["ownerQualifiedName"],
                relation["targetSpelling"],
            )
        )
        normalized_files.append(
            {
                "path": relative,
                "status": status,
                "bytes": (root / relative).stat().st_size,
                "relations": normalized_relations,
            }
        )
        observed_paths.append(relative)
    if observed_paths != relative_paths or observed_paths != sorted(set(observed_paths)):
        raise OracleError(f"{language} parser provider did not return the complete sorted inventory")
    partial = sum(item["status"] == "partial" for item in normalized_files)
    inventory = {
        "language": language,
        "provider": provider,
        "toolchain": document["toolchain"],
        "rootRelativeFiles": observed_paths,
        "files": normalized_files,
    }
    return {
        "schema": f"compass.{language}-source-oracle/1",
        "language": language,
        "provider": provider,
        "toolchain": document["toolchain"],
        "implementation": document["implementation"],
        "parserAvailable": True,
        "limits": {
            "maxFiles": MAX_FILES,
            "maxFileBytes": MAX_FILE_BYTES,
            "maxTotalBytes": MAX_TOTAL_BYTES,
        },
        "scannedFiles": len(normalized_files),
        "parsedFiles": len(normalized_files) - partial,
        "partialFiles": partial,
        "inventorySha256": digest(inventory),
        "files": normalized_files,
    }


def run_oracle_with_provider(
    root: Path,
    *,
    language: str,
    provider: str,
    toolchain: str,
    implementation: str,
    suffixes: tuple[str, ...],
    include_globs: tuple[str, ...] = (),
    exclude_globs: tuple[str, ...] = (),
) -> dict[str, Any]:
    command = _provider_command(language)
    if command is not None:
        if not command.is_file() or not os.access(command, os.X_OK):
            raise OracleError(f"configured {language} parser provider is not executable: {command}")
        return _run_parser_provider(
            root,
            language=language,
            provider=provider,
            suffixes=suffixes,
            include_globs=include_globs,
            exclude_globs=exclude_globs,
            command=command,
        )
    return run_oracle(
        root,
        language=language,
        provider=provider,
        toolchain=toolchain,
        implementation=implementation,
        parser_available=False,
        suffixes=suffixes,
        include_globs=include_globs,
        exclude_globs=exclude_globs,
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--language", required=True, choices=("swift", "dart", "scala", "groovy"))
    parser.add_argument("--provider", required=True)
    parser.add_argument("--toolchain", required=True)
    parser.add_argument("--suffix", action="append", required=True)
    parser.add_argument("--include", action="append", default=[])
    parser.add_argument("--exclude", action="append", default=[])
    args = parser.parse_args()
    try:
        payload = run_oracle(
            args.root,
            language=args.language,
            provider=args.provider,
            toolchain=args.toolchain,
            suffixes=tuple(sorted(set(s.casefold() for s in args.suffix))),
            include_globs=tuple(args.include),
            exclude_globs=tuple(args.exclude),
        )
        encoded = canonical_bytes(payload)
        if args.output:
            args.output.write_bytes(encoded)
        else:
            print(encoded.decode("utf-8"), end="")
        return 0
    except (OSError, OracleError) as error:
        print(f"{args.language} source oracle failed: {error}", file=__import__("sys").stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
