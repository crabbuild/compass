"""Bounded, optional Go source-candidate oracle; never an adjudication source."""

from __future__ import annotations

from collections import Counter
from bisect import bisect_right
import hashlib
import json
import os
from pathlib import Path, PurePosixPath, PureWindowsPath
import re
import struct
import subprocess
import threading
import time

from .occurrences import (
    SourceConstruct, SourceConstructInventory, _qualification_glob_matches,
    _source_construct_key,
)

PROVIDER = "go_stdlib_ast_types_v1"
SCHEMA = "compass.go-source-oracle/1"
HELPER = Path(__file__).resolve().parents[1] / "oracles" / "go-source-oracle.go"
MAX_FILES = 20_000
MAX_SOURCE_BYTES = 128 * 1024 * 1024
MAX_RECORDS = 500_000
MAX_DIAGNOSTICS = 10_000
MAX_REQUEST_BYTES = 8 * 1024 * 1024
MAX_OUTPUT_BYTES = 64 * 1024 * 1024
TIMEOUT_SECONDS = 90.0
STATUSES = frozenset((
    "local_concrete", "external", "interface_dispatch", "function_value",
    "builtin", "type_conversion", "unresolved_call_or_conversion",
))


def _path(value: object) -> str:
    if not isinstance(value, str) or not value or len(value.encode()) > 4096 or "\\" in value or any(ord(c) < 32 for c in value):
        raise RuntimeError("Go oracle path is not a canonical relative path")
    p = PurePosixPath(value)
    if p.is_absolute() or PureWindowsPath(value).drive or ".." in p.parts or p.as_posix() != value:
        raise RuntimeError("Go oracle path escapes the source root")
    return value


def _fields(value: object, expected: set[str], context: str) -> dict:
    if not isinstance(value, dict) or set(value) != expected:
        raise RuntimeError(f"Go oracle {context} has invalid fields")
    return value


def _integer(value: object, maximum: int) -> int:
    if type(value) is not int or not 0 <= value <= maximum:
        raise RuntimeError("Go oracle integer is outside its bounds")
    return value


def _text(value: object) -> str:
    if not isinstance(value, str) or not value or len(value.encode()) > 4096 or any(ord(c) < 32 for c in value):
        raise RuntimeError("Go oracle identity text is invalid")
    return value


def _inventory_digest(files: list[dict]) -> str:
    digest = hashlib.sha256()
    for file in files:
        for value in (file["path"], file["sha256"], str(file["byteSize"])):
            raw = value.encode()
            digest.update(struct.pack("<Q", len(raw)))
            digest.update(raw)
    return digest.hexdigest()


def _inputs(root: Path, includes: tuple[str, ...], excludes: tuple[str, ...]) -> tuple[dict, dict[str, bytes]]:
    root = root.resolve(strict=True)
    if not root.is_dir():
        raise RuntimeError("Go source root is not a directory")
    sources: dict[str, bytes] = {}
    total = directories = 0
    for directory, names, files in os.walk(root, followlinks=False):
        directories += 1
        if directories > MAX_FILES:
            raise RuntimeError("Go source directory limit exceeded")
        names[:] = sorted(name for name in names if name != ".git")
        for name in names:
            if (Path(directory) / name).is_symlink():
                raise RuntimeError("Go source inventory contains a symlink directory")
        for name in sorted(files):
            if not name.endswith(".go"):
                continue
            p = Path(directory) / name
            relative = _path(p.relative_to(root).as_posix())
            if includes and not any(_qualification_glob_matches(relative, pattern) for pattern in includes):
                continue
            if any(_qualification_glob_matches(relative, pattern) for pattern in excludes):
                continue
            if p.is_symlink() or not p.is_file():
                raise RuntimeError("Go source inventory contains a nonregular file")
            p.resolve(strict=True).relative_to(root)
            size = p.stat().st_size
            if size > MAX_SOURCE_BYTES - total or len(sources) >= MAX_FILES:
                raise RuntimeError("Go source inventory limit exceeded")
            with p.open("rb") as source:
                raw = source.read(size + 1)
            if len(raw) != size:
                raise RuntimeError("Go source changed while reading")
            sources[relative] = raw
            total += size
    sources = dict(sorted(sources.items()))
    if not sources:
        raise RuntimeError("Go source inventory has no selected files")
    files = [dict(path=p, sha256=hashlib.sha256(raw).hexdigest(), byteSize=len(raw)) for p, raw in sources.items()]
    return dict(root=str(root), files=files), sources


def _run(binary: Path, request: bytes) -> bytes:
    if len(request) > MAX_REQUEST_BYTES:
        raise RuntimeError("Go source oracle request byte limit exceeded")
    env = dict(os.environ, GOPROXY="off", GOSUMDB="off", GOTOOLCHAIN="local", GOWORK="off", GOENV="off")
    process = subprocess.Popen(
        [str(binary)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        cwd=HELPER.parent, env=env, shell=False,
    )
    buffers = [bytearray(), bytearray()]
    overflow = threading.Event()
    failures: list[BaseException] = []

    def read(pipe, destination: bytearray, limit: int) -> None:
        try:
            while chunk := pipe.read(64 * 1024):
                if len(destination) + len(chunk) > limit:
                    overflow.set()
                    return
                destination.extend(chunk)
        except (OSError, ValueError) as error:
            failures.append(error)
        finally:
            pipe.close()

    def write() -> None:
        try:
            process.stdin.write(request)
        except (BrokenPipeError, OSError, ValueError):
            pass
        finally:
            try:
                process.stdin.close()
            except (BrokenPipeError, OSError, ValueError):
                pass

    assert process.stdout is not None and process.stderr is not None and process.stdin is not None
    threads = [
        threading.Thread(target=read, args=(process.stdout, buffers[0], MAX_OUTPUT_BYTES), daemon=True),
        threading.Thread(target=read, args=(process.stderr, buffers[1], 64 * 1024), daemon=True),
        threading.Thread(target=write, daemon=True),
    ]
    for thread in threads:
        thread.start()
    deadline = time.monotonic() + TIMEOUT_SECONDS
    timed_out = False
    try:
        while process.poll() is None:
            if overflow.is_set() or failures:
                break
            if time.monotonic() >= deadline:
                timed_out = True
                break
            time.sleep(0.01)
    finally:
        if process.poll() is None:
            process.kill()
        process.wait()
        for thread in threads:
            thread.join(timeout=2)
    if any(thread.is_alive() for thread in threads):
        raise RuntimeError("Go source oracle pipe did not close")
    if timed_out:
        raise RuntimeError("Go source oracle timed out")
    if overflow.is_set():
        raise RuntimeError("Go source oracle output byte limit exceeded")
    if failures or process.returncode:
        # Do not reproduce untrusted subprocess text or source literals.
        raise RuntimeError("Go source oracle failed; no partial inventory accepted")
    return bytes(buffers[0])


def _boundary(raw: bytes, at: int) -> bool:
    return at == len(raw) or raw[at] & 0xC0 != 0x80


def _anchor(value: dict, raw: bytes, lines: list[int]) -> tuple[int, int, int]:
    start = _integer(value["startByte"], len(raw))
    end = _integer(value["endByte"], len(raw))
    line = _integer(value["startLine"], len(raw) + 1)
    if start >= end or line < 1 or line != bisect_right(lines, start):
        raise RuntimeError("Go oracle source anchor is inconsistent")
    if not _boundary(raw, start) or not _boundary(raw, end):
        raise RuntimeError("Go oracle source anchor splits UTF-8")
    return start, end, line


def validate_payload(request: dict, sources: dict[str, bytes], payload: object) -> SourceConstructInventory:
    p = _fields(payload, {"schema", "provider", "toolchain", "platform", "selectionPolicy", "parserAvailable", "sourceDigest", "files", "diagnostics", "counts"}, "payload")
    if p["schema"] != SCHEMA or p["provider"] != PROVIDER or p["parserAvailable"] is not True:
        raise RuntimeError("Go oracle schema/provider mismatch")
    if p["selectionPolicy"] != "all_selected_source_no_build_filter" or not re.fullmatch(r"go1\.\d+(?:\.\d+)?(?:[a-z]+\d+)?", _text(p["toolchain"])):
        raise RuntimeError("Go oracle selection/toolchain mismatch")
    if not re.fullmatch(r"[a-z0-9]+/[a-z0-9]+", _text(p["platform"])):
        raise RuntimeError("Go oracle platform metadata is invalid")
    if p["sourceDigest"] != _inventory_digest(request["files"]):
        raise RuntimeError("Go oracle source digest mismatch")
    files = p["files"]
    if not isinstance(files, list) or len(files) != len(sources) or len(files) > MAX_FILES:
        raise RuntimeError("Go oracle file coverage mismatch")
    if [f.get("path") if isinstance(f, dict) else None for f in files] != list(sources):
        raise RuntimeError("Go oracle file ledger mismatch")
    expected = {f["path"]: f for f in request["files"]}
    declarations: set[tuple] = set()
    constructs: list[SourceConstruct] = []
    statuses: Counter = Counter()
    counts = dict(files=len(files), parsed=0, declarations=0, calls=0, imports=0, diagnostics=0)
    rejected = []
    typed_files = set()
    line_offsets = {name: [0, *(match.end() for match in re.finditer(b"\n", raw))] for name, raw in sources.items()}
    anchor_keys = {"startByte", "endByte", "startLine"}
    for file in files:
        _fields(file, {"path", "sha256", "byteSize", "status", "typeCheckComplete", "declarations", "calls", "imports"}, "file")
        name = _path(file["path"])
        raw = sources[name]
        if file["sha256"] != expected[name]["sha256"] or type(file["byteSize"]) is not int or file["byteSize"] != len(raw):
            raise RuntimeError("Go oracle file identity mismatch")
        if not isinstance(file["status"], str) or file["status"] not in {"parsed", "rejected"} or type(file["typeCheckComplete"]) is not bool:
            raise RuntimeError("Go oracle file status is invalid")
        for key in ("declarations", "calls", "imports"):
            if not isinstance(file[key], list):
                raise RuntimeError("Go oracle record list is invalid")
            counts[key] += len(file[key])
        if sum(counts[k] for k in ("declarations", "calls", "imports")) > MAX_RECORDS:
            raise RuntimeError("Go oracle source record limit exceeded")
        if file["status"] == "rejected":
            if file["typeCheckComplete"] or any(file[k] for k in ("declarations", "calls", "imports")):
                raise RuntimeError("Go oracle rejected file contains accepted records")
            rejected.append(name)
            continue
        counts["parsed"] += 1
        try:
            raw.decode("utf-8")
        except UnicodeError as error:
            raise RuntimeError("Go oracle parsed invalid UTF-8") from error
        if file["typeCheckComplete"]:
            typed_files.add(name)
        for decl in file["declarations"]:
            _fields(decl, anchor_keys | {"name", "qualifiedName", "kind", "bodyStartByte", "bodyEndByte"}, "declaration")
            a = _anchor(decl, raw, line_offsets[name])
            symbol = _text(decl["name"])
            qualified = _text(decl["qualifiedName"])
            start = _integer(decl["bodyStartByte"], len(raw))
            end = _integer(decl["bodyEndByte"], len(raw))
            if not isinstance(decl["kind"], str) or decl["kind"] not in {"function", "method", "closure"} or not start <= a[0] < a[1] <= end or not _boundary(raw, start) or not _boundary(raw, end):
                raise RuntimeError("Go oracle declaration body is inconsistent")
            if decl["kind"] != "closure" and raw[a[0]:a[1]].decode() != symbol:
                raise RuntimeError("Go oracle declaration spelling is inconsistent")
            identity = (name, qualified, *a)
            if identity in declarations:
                raise RuntimeError("Go oracle duplicate declaration")
            declarations.add(identity)
    for file in files:
        name, raw = file["path"], sources[file["path"]]
        seen_calls: set[tuple] = set()
        # Interval events validate the narrowest enclosing callable without a
        # quadratic declaration scan for every call. A literal invocation
        # belongs outside the literal body; its inner calls belong inside it.
        events = [(d["bodyStartByte"], 0, -d["bodyEndByte"], d["qualifiedName"]) for d in file["declarations"]]
        for item in file["calls"]:
            _fields(item, anchor_keys | {"owner", "spelling", "qualifier", "invocationStartByte", "invocationEndByte", "bindingStatus", "target"}, "call")
            start, end, line = _anchor(item, raw, line_offsets[name])
            inv_start = _integer(item["invocationStartByte"], len(raw))
            inv_end = _integer(item["invocationEndByte"], len(raw))
            status = item["bindingStatus"]
            if not isinstance(status, str) or status not in STATUSES or not inv_start <= start < end <= inv_end or not _boundary(raw, inv_start) or not _boundary(raw, inv_end):
                raise RuntimeError("Go oracle call status/range is inconsistent")
            owner, spelling = _text(item["owner"]), _text(item["spelling"])
            if (owner, start, end) in seen_calls:
                raise RuntimeError("Go oracle duplicate call expression")
            seen_calls.add((owner, start, end))
            events.append((inv_start, 1, -inv_end, owner))
            qualifier = _text(item["qualifier"]) if item["qualifier"] != "" else None
            if status == "local_concrete":
                target = _fields(item["target"], anchor_keys | {"file", "qualifiedName"}, "target")
                target_file = _path(target["file"])
                if target_file not in sources or name not in typed_files or target_file not in typed_files:
                    raise RuntimeError("Go oracle concrete target lacks complete types")
                a = _anchor(target, sources[target_file], line_offsets[target_file])
                if (target_file, _text(target["qualifiedName"]), *a) not in declarations:
                    raise RuntimeError("Go oracle concrete target lacks a declaration")
            elif item["target"] is not None:
                raise RuntimeError("Go oracle nonconcrete status has a target")
            statuses[status] += 1
            if status != "type_conversion":
                constructs.append(SourceConstruct(name, "calls", "calls", owner, spelling, qualifier, start, end, line))
        stack: list[tuple[int, str]] = []
        negative_ends: list[int] = []
        for start, kind, negative_end, owner in sorted(events):
            while stack and start >= stack[-1][0]:
                stack.pop()
                negative_ends.pop()
            if kind == 0:
                if stack and -negative_end > stack[-1][0]:
                    raise RuntimeError("Go oracle declaration scopes cross")
                stack.append((-negative_end, owner))
                negative_ends.append(negative_end)
            else:
                index = bisect_right(negative_ends, negative_end) - 1
                expected_owner = stack[index][1] if index >= 0 else name
                if owner != expected_owner:
                    raise RuntimeError("Go oracle call owner is not the enclosing declaration")
        for item in file["imports"]:
            _fields(item, anchor_keys | {"owner", "path", "alias"}, "import")
            start, end, line = _anchor(item, raw, line_offsets[name])
            path, owner = _text(item["path"]), _text(item["owner"])
            alias = _text(item["alias"]) if item["alias"] != "" else None
            constructs.append(SourceConstruct(name, "imports", "imports", owner, path, alias, start, end, line))
    diagnostics = p["diagnostics"]
    if not isinstance(diagnostics, list) or len(diagnostics) > MAX_DIAGNOSTICS:
        raise RuntimeError("Go oracle diagnostic limit exceeded")
    for item in diagnostics:
        _fields(item, {"file", "line", "code"}, "diagnostic")
        if _path(item["file"]) not in sources or not isinstance(item["code"], str) or item["code"] not in {"parse_error", "types_error"}:
            raise RuntimeError("Go oracle diagnostic is inconsistent")
        if _integer(item["line"], len(sources[item["file"]]) + 1) < 1:
            raise RuntimeError("Go oracle diagnostic line is invalid")
    counts["diagnostics"] = len(diagnostics)
    if p["counts"] != counts or any(type(v) is not int for v in p["counts"].values()):
        raise RuntimeError("Go oracle record counts mismatch")
    if len(set(constructs)) != len(constructs):
        raise RuntimeError("Go oracle duplicate source occurrence")
    metadata = (
        ("oracleImplementation", "go_stdlib_ast_types_source_candidates"),
        ("parserAvailable", "true"),
        ("oracleToolchain", p["toolchain"]),
        ("oraclePlatform", p["platform"]),
        ("oracleSourceDigest", p["sourceDigest"]),
        ("selectionPolicy", p["selectionPolicy"]),
        ("importPolicy", "deny_external_exports_no_subprocess"),
        ("allTypesAvailable", str(len(typed_files) == len(files)).lower()),
        ("bindingStatuses", json.dumps(dict(sorted(statuses.items())), separators=(",", ":"))),
        ("diagnosticCount", str(len(diagnostics))),
        ("oraclePayloadSha256", hashlib.sha256(json.dumps(p, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()).hexdigest()),
    )
    return SourceConstructInventory(tuple(sorted(constructs, key=_source_construct_key)), len(files), counts["parsed"], tuple(rejected), tuple(sorted(metadata)))


def collect_go_inventory(root: Path, include_globs: tuple[str, ...] = (), exclude_globs: tuple[str, ...] = ()) -> SourceConstructInventory:
    configured = os.environ.get("COMPASS_GO_SOURCE_ORACLE")
    if not configured or not Path(configured).is_absolute():
        raise RuntimeError("Go qualification needs an explicitly built COMPASS_GO_SOURCE_ORACLE binary")
    binary = Path(configured).resolve(strict=True)
    if not binary.is_file() or not os.access(binary, os.X_OK) or binary.stat().st_size > 256 * 1024 * 1024:
        raise RuntimeError("Go qualification binary is invalid")
    request, sources = _inputs(root, include_globs, exclude_globs)
    encoded = json.dumps(request, separators=(",", ":"), ensure_ascii=False).encode()
    identity = binary.stat()
    digest = hashlib.sha256()
    with binary.open("rb") as executable:
        remaining = 256 * 1024 * 1024
        while chunk := executable.read(min(64 * 1024, remaining + 1)):
            remaining -= len(chunk)
            if remaining < 0:
                raise RuntimeError("Go qualification binary changed while hashing")
            digest.update(chunk)
    raw = _run(binary, encoded)
    observed = binary.stat()
    if (identity.st_ino, identity.st_size, identity.st_mtime_ns) != (observed.st_ino, observed.st_size, observed.st_mtime_ns):
        raise RuntimeError("Go qualification binary changed during execution")
    def unique_object(pairs):
        value = {}
        for key, item in pairs:
            if key in value:
                raise ValueError("duplicate JSON object field")
            value[key] = item
        return value
    def invalid_constant(_value):
        raise ValueError("nonfinite JSON constant")
    try:
        payload = json.loads(raw, object_pairs_hook=unique_object, parse_constant=invalid_constant)
    except (ValueError, UnicodeError) as error:
        raise RuntimeError("Go oracle returned invalid JSON") from error
    result = validate_payload(request, sources, payload)
    metadata = dict(result.provider_metadata)
    metadata.update(oracleBinarySha256=digest.hexdigest(), oracleHelperSha256=hashlib.sha256(HELPER.read_bytes()).hexdigest())
    return SourceConstructInventory(result.constructs, result.scanned_files, result.parsed_files, result.rejected_files, tuple(sorted(metadata.items())))
