"""Explicit developer protocol2 capture, scoring and qualification checking.

Version1 commands remain separate. Capture uses preinstalled native tools and
prebuilt graphs only; score/check execute no tool query. A coherent capture or
diagnostic report does not establish comparative quality.
"""
import argparse
from pathlib import Path
import subprocess
import sys

from benchmarks.agent_query.broad_query_collect_v2 import collect_file_v2
from benchmarks.agent_query.broad_query_report_v2 import score_file_v2, check_file_v2, encode_report


def _status(assessment):
    failures = assessment["failures"]
    return dict(eligibleForSuperiorityClaim=assessment["eligibleForSuperiorityClaim"],
        failureCount=len(failures), failures=failures[:10], additionalFailures=max(0, len(failures) - 10),
        panelEligibility=assessment.get("panelAssessment", {}).get("panelEligibility"))


def _emit(value):
    print(encode_report(value).decode("utf-8"), end="")


def _error_preview(error):
    # Bound characters before encoding/escaping; one scalar can otherwise
    # allocate its entire hostile value before the byte limit is applied.
    prefix = str(error)[:2048]
    escaped = "".join(character if character.isprintable() else
        "\\u%04x" % ord(character) for character in prefix)
    return escaped.encode("utf-8", errors="replace")[:2048].decode("utf-8", errors="ignore")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    action = parser.add_mutually_exclusive_group(required=True)
    action.add_argument("--capture", type=Path, help="frozen capture-input2; execute the complete native matrix once")
    action.add_argument("--score", type=Path, help="review-input2; recompute union and frozen original/lookup panels")
    action.add_argument("--check", type=Path, help="report2; recompute and require every quality/qualification gate")
    action.add_argument("--self-test", action="store_true", help="run synthetic protocol2 contracts, not qualification")
    parser.add_argument("--output", type=Path, help="new capture directory or new sibling report; never overwritten")
    args = parser.parse_args(argv)
    if (args.capture or args.score) and args.output is None:
        parser.error("--capture and --score require --output")
    if (args.check or args.self_test) and args.output is not None:
        parser.error("--output is supported only with --capture or --score")
    try:
        if args.self_test:
            import unittest
            suite = unittest.defaultTestLoader.loadTestsFromNames([
                "benchmarks.agent_query.tests.test_broad_query_actors_v2",
                "benchmarks.agent_query.tests.test_broad_query_lineage_v2",
                "benchmarks.agent_query.tests.test_broad_query_collect_v2",
                "benchmarks.agent_query.tests.test_broad_query_report_v2",
                "benchmarks.agent_query.tests.test_broad_query_panels_v2",
                "benchmarks.agent_query.tests.test_broad_query_v2",
            ])
            return 0 if unittest.TextTestRunner(verbosity=1).run(suite).wasSuccessful() else 1
        if args.capture:
            _emit(collect_file_v2(args.capture, args.output))
            return 0
        if args.score:
            report = score_file_v2(args.score, args.output)
            _emit(dict(report=str(args.output.absolute()), **_status(report["assessment"])))
            return 0  # Diagnostic scoring can succeed while qualification fails.
        assessment = check_file_v2(args.check)
        _emit(_status(assessment))
        return 0 if assessment["eligibleForSuperiorityClaim"] else 1
    except (OSError, ValueError, TypeError, KeyError, IndexError, RuntimeError, subprocess.SubprocessError) as error:
        # Bound untrusted path/metadata text on the public error stream too.
        detail = _error_preview(error)
        print("broad-query protocol2 rejected: " + detail, file=sys.stderr)
        return 2


if __name__ == "__main__": raise SystemExit(main())
