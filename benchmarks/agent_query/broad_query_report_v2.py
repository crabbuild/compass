"""Internal protocol2 report publication and exact recomputation.

The explicit version2 developer entry invokes these publication/check paths.
Reports preserve the original union run and finite reviewed selections; source
and reviewer authenticity remain independent qualification obligations.
"""
import json
import math
from pathlib import Path

from benchmarks.agent_query import broad_query_review as old
from benchmarks.agent_query.broad_query_assess_v2 import _assessment_session
from benchmarks.agent_query.broad_query_lineage_v2 import SCHEMAS
from benchmarks.agent_query.broad_query_oracle import publish_exclusive


def encode_report(value):
    """Canonical indent2/sorted/ASCII JSON with bounded scalar allocation.

    String escaping is preflighted without constructing its escaped token,
    then emitted in <=1024-character source chunks. There is no stricter string
    limit: exact total encoded bytes, including the final LF, decide capacity.
    """
    data = bytearray()
    active = set()

    def remaining():
        return old.MAX_JSON_BYTES - len(data) - 1  # reserve final LF

    def require(size):
        if size > remaining():
            raise ValueError("report exceeds bounded JSON artifact size")

    def emit(text):
        require(len(text))  # all emitted text is ASCII
        data.extend(text.encode("ascii"))

    def string(text):
        require(len(text) + 2)  # every source character needs at least one byte
        size = 2
        for char in text:
            code = ord(char)
            size += (2 if char in '\\"\b\f\n\r\t' else 1 if 32 <= code < 127
                else 6 if code <= 65535 else 12)
            require(size)
        emit('"')
        for offset in range(0, len(text), 1024):
            # The complete scalar fits before any escaping. Each temporary
            # token is bounded by 1024*12+2 ASCII bytes, even for surrogates.
            emit(json.encoder.encode_basestring_ascii(text[offset:offset + 1024])[1:-1])
        emit('"')

    def integer(number):
        negative = number < 0
        absolute = -number if negative else number
        bits = absolute.bit_length()
        budget = remaining() - int(negative)
        lower = 1 if not bits else (bits - 1) * 3 // 10 + 1
        if lower > budget:
            require(remaining() + 1)
        # 30103/100000 is a strict upper bound on log10(2). Only the
        # inconclusive boundary needs an exact power comparison; small typed
        # metrics do not allocate a power proportional to the report allowance.
        upper = 1 if not bits else (bits * 30103 + 99999) // 100000
        if upper > budget and absolute >= 10**budget:
            require(remaining() + 1)
        emit(str(number))

    def indent(depth):
        require(2 * depth)
        emit(" " * (2 * depth))

    def encode(item, depth=0):
        kind = type(item)
        if item is None: emit("null")
        elif kind is bool: emit("true" if item else "false")
        elif kind is str: string(item)
        elif kind is int: integer(item)
        elif kind is float:
            if not math.isfinite(item): raise ValueError("nonfinite report number")
            emit(repr(item))  # finite IEEE754 repr is a fixed small scalar
        elif kind in (dict, list, tuple):
            identity = id(item)
            if identity in active: raise ValueError("circular report value")
            active.add(identity)
            try:
                is_dict = kind is dict
                if is_dict and any(type(key) is not str for key in item):
                    raise TypeError("report object keys must be strings")
                # A conservative lower bound also bounds temporary key sorting.
                require(2 + len(item) * (4 if is_dict else 1))
                emit("{" if is_dict else "[")
                if item:
                    emit("\n")
                    for index, key in enumerate(sorted(item) if is_dict else range(len(item))):
                        if index: emit(",\n")
                        indent(depth + 1)
                        if is_dict:
                            string(key)
                            emit(": ")
                        encode(item[key], depth + 1)
                    emit("\n")
                    indent(depth)
                emit("}" if is_dict else "]")
            finally:
                active.remove(identity)
        else:
            raise TypeError("report requires native JSON values")

    encode(value)
    data.extend(b"\n")
    return bytes(data)


def _input(path):
    path = Path(path).absolute()
    data = old.read_bounded(path, old.MAX_JSON_BYTES)
    return path, data, old._json(data)


def _unchanged(path, data):
    # Use the verified length, rather than another full per-file allowance.
    if old.read_bounded(path, len(data)) != data:
        raise ValueError("report input changed during assessment or publication")


def score_file_v2(path, output, *, _publish=None):
    """Publish one no-clobber report after final source/input/actor guards.

    Version2 reports are siblings of their input in the external evidence root.
    Encoding failure or any final integrity failure publishes no report.
    Diagnostic publication does not mean its qualification gates passed.
    """
    path, data, manifest = _input(path)
    output = Path(output).absolute()
    if output.parent.resolve() != path.parent.resolve():
        raise ValueError("version2 reports must be siblings of their review input")
    old._relative(output.name)
    if output.exists() or output.is_symlink():
        raise FileExistsError("report output already exists")
    with _assessment_session(manifest, path.parent) as (assessment, recheck):
        report = dict(schema=SCHEMAS["review"], inputPath=str(path), inputSha256=old.sha(data),
            actorSha256=assessment["actorSha256"], assessment=assessment)
        encoded = encode_report(report)
        recheck()
        _unchanged(path, data)
        if output.parent.resolve() != path.parent.resolve():
            raise ValueError("report output parent changed during publication")
        (publish_exclusive if _publish is None else _publish)(output, encoded)
    return report


def recompute_report_v2(report):
    """Never trust stored metrics, completeness or an asserted eligibility bit."""
    if (type(report) is not dict or report.keys() != {"schema", "inputPath", "inputSha256", "actorSha256", "assessment"}
            or report["schema"] != SCHEMAS["review"]):
        raise ValueError("unsupported or mixed version2 report schema")
    report = old._json(encode_report(report))
    path = Path(old._text(report["inputPath"], "absolute report input path", 4096))
    if not path.is_absolute():
        raise ValueError("version2 report input must be absolute")
    expected = old._digest(report["inputSha256"])
    path, data, manifest = _input(path)
    if old.sha(data) != expected:
        raise ValueError("report input differs from its original bytes")
    with _assessment_session(manifest, path.parent) as (assessment, recheck):
        if report["actorSha256"] != assessment["actorSha256"]:
            raise ValueError("report actors differ from the complete frozen protocol")
        # Canonical bytes reject bool/int and int/float substitutions too.
        if encode_report(report["assessment"]) != encode_report(assessment):
            raise ValueError("stored metrics or claim eligibility were fabricated or are stale")
        recheck()
        _unchanged(path, data)
    return assessment


def check_file_v2(path):
    path, data, report = _input(path)
    assessment = recompute_report_v2(report)
    _unchanged(path, data)
    return assessment
