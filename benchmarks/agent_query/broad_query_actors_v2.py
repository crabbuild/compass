"""Freeze the complete owned evaluator source set for protocol2 experiments.

This checks supplied code-byte integrity, not reviewer independence or source
truth. It does not dispatch capture/assessment or authorize an experiment.
"""
import json
import os
from pathlib import Path

from benchmarks.agent_query.broad_query_review import (
    MAX_JSON_BYTES, _digest, _json, _relative, read_bounded, sha,
)

SCHEMA = "compass.broad-query-protocol-source/2"
BINDING_SCHEMA = "compass.broad-query-protocol-binding/2"
ROOTS = ("benchmarks/agent_query", "benchmarks/performance/compass")
ROLES = dict(capture="benchmarks/agent_query/broad_query_v2.py",
    normalizer="benchmarks/agent_query/broad_query_normalize_v2.py",
    assessor="benchmarks/agent_query/broad_query_assess_v2.py",
    panel="benchmarks/agent_query/broad_query_panels_v2.py",
    reporter="benchmarks/agent_query/broad_query_report_v2.py")
MAX_FILES = 512
MAX_BYTES = MAX_JSON_BYTES
MAX_TREE_ENTRIES = 16384
MAX_TREE_DIRECTORIES = 512


def _encoded(document):
    data = (json.dumps(document, indent=2, sort_keys=True, allow_nan=False) + "\n").encode()
    if len(data) > MAX_JSON_BYTES:
        raise ValueError("protocol source descriptor exceeds JSON byte limit")
    return data


def _module(value):
    relative = _relative(value)
    path = Path(relative)
    if (not relative.endswith(".py") or "tests" in path.parts
            or not any(relative.startswith(root + "/") for root in ROOTS)):
        raise ValueError("protocol actor is outside the owned evaluator source set")
    return relative


def _descriptor(document):
    if (type(document) is not dict or document.keys() != {"schema", "roles", "files"}
            or document["schema"] != SCHEMA or document["roles"] != ROLES
            or type(document["files"]) is not list or not 1 <= len(document["files"]) <= MAX_FILES):
        raise ValueError("invalid protocol source descriptor")
    files, names = {}, []
    for item in document["files"]:
        if type(item) is not dict or item.keys() != {"module", "sha256"}:
            raise ValueError("invalid protocol source file record")
        module = _module(item["module"])
        if module in files:
            raise ValueError("duplicate protocol source module")
        files[module] = _digest(item["sha256"])
        names.append(module)
    if names != sorted(names) or not set(ROLES.values()) <= files.keys():
        raise ValueError("protocol source order or required actor roles differ")
    actors = {role: files[module] for role, module in ROLES.items()}
    actors["protocol"] = sha(_encoded(document))
    return files, dict(sorted(actors.items()))


def freeze_protocol(root):
    """Hash all non-test Python files under both fixed evaluator source roots.

    The descriptor contains file digests, not source text. Freezing the whole
    source set includes imported helpers and graph auditors; adding/removing
    even an unused owned source file requires a new pre-output freeze.
    """
    root = Path(root).resolve(strict=True)
    paths, pending = [], []
    for directory in ROOTS:
        base = root / directory
        if not base.is_dir() or base.resolve() != base:
            raise ValueError("protocol source root is absent or follows a symlink")
        pending.append(base)
    entries, directories = 0, len(pending)
    if directories > MAX_TREE_DIRECTORIES:
        raise ValueError("protocol source directory limit exceeded")
    while pending:
        with os.scandir(pending.pop()) as children:
            for child in children:
                entries += 1
                if entries > MAX_TREE_ENTRIES:
                    raise ValueError("protocol source tree entry limit exceeded")
                if child.name == "tests" and child.is_dir(follow_symlinks=False):
                    continue
                if child.is_symlink():
                    raise ValueError("protocol source tree cannot contain symlinks")
                path = Path(child.path)
                if child.is_dir(follow_symlinks=False):
                    if directories >= MAX_TREE_DIRECTORIES:
                        raise ValueError("protocol source directory limit exceeded")
                    directories += 1
                    pending.append(path)
                elif child.name.endswith(".py"):
                    if len(paths) >= MAX_FILES:
                        raise ValueError("protocol source file count exceeds512")
                    if path.resolve() != path or not child.is_file(follow_symlinks=False):
                        raise ValueError("protocol source must be a regular contained file")
                    paths.append((path.relative_to(root).as_posix(), path))
    files, total = [], 0
    for relative, path in sorted(paths):
        data = read_bounded(path, MAX_BYTES - total)
        total += len(data)
        files.append(dict(module=_module(relative), sha256=sha(data)))
    descriptor = dict(schema=SCHEMA, roles=dict(ROLES), files=files)
    _, actors = _descriptor(descriptor)
    return descriptor, actors


def verify_original_protocol(root, descriptor, actors):
    _, expected = _descriptor(descriptor)
    if type(actors) is not dict or actors != expected:
        raise ValueError("frozen actor tuple differs from protocol source descriptor")
    actual_descriptor, actual_actors = freeze_protocol(root)
    if actual_descriptor != descriptor or actual_actors != actors:
        raise ValueError("original evaluator source inventory or bytes drifted")


def retain_protocol(bundle, root, descriptor, actors):
    """Retain hash-bound code copies using the existing no-clobber bundle."""
    verify_original_protocol(root, descriptor, actors)
    copies = []
    for item in descriptor["files"]:
        module = item["module"]
        reference = bundle.copy("provenance/protocol/" + module,
            dict(path=str(Path(root).resolve() / module), sha256=item["sha256"]))
        copies.append(dict(module=module, artifact=reference))
    manifest = bundle.put("provenance/protocol-source.json", descriptor)
    if manifest["sha256"] != actors["protocol"]:
        raise ValueError("retained protocol descriptor encoding differs")
    verify_original_protocol(root, descriptor, actors)
    return dict(schema=BINDING_SCHEMA, protocolManifest=manifest, files=copies, actorSha256=actors)


def verify_retained_protocol(binding, artifacts, root):
    """Verify retained and original actors with one shared bounded reader.

    The runtime caller supplies its own product code root, never a root selected
    from an untrusted capture/manifest. Call before assessment and publication.
    The caller additionally
    rechecks all raw/source/run/normalization artifacts with this same reader.
    """
    if (type(binding) is not dict or binding.keys() != {"schema", "protocolManifest", "files", "actorSha256"}
            or binding["schema"] != BINDING_SCHEMA):
        raise ValueError("invalid retained protocol binding")
    descriptor = _json(artifacts.read(binding["protocolManifest"]))
    files, actors = _descriptor(descriptor)
    if binding["actorSha256"] != actors or binding["protocolManifest"]["sha256"] != actors["protocol"]:
        raise ValueError("retained evaluator actor tuple differs")
    copies = binding["files"]
    if type(copies) is not list or len(copies) != len(files):
        raise ValueError("retained protocol source partition differs")
    verify_original_protocol(root, descriptor, actors)
    total = 0
    for copy, (module, digest) in zip(copies, files.items()):
        if (type(copy) is not dict or copy.keys() != {"module", "artifact"}
                or copy["module"] != module or type(copy["artifact"]) is not dict
                or copy["artifact"].get("sha256") != digest):
            raise ValueError("retained protocol module order or digest differs")
        data = artifacts.read(copy["artifact"], MAX_BYTES - total)
        total += len(data)
    verify_original_protocol(root, descriptor, actors)
    return descriptor, actors
