"""Inclusive input limits with spies; no large workloads or native queries."""
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from benchmarks.agent_query import broad_query_capture as capture
from benchmarks.agent_query import broad_query_inputs_v2 as inputs
from benchmarks.agent_query import broad_query_review as old
from benchmarks.agent_query.tests.test_broad_query_assess_v2 import Fixture


class InputBoundsTests(unittest.TestCase):
    def test_original_and_retained_unique_count_excess_precedes_read_or_hash(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "a").write_bytes(b"ab")
            (root / "b").write_bytes(b"c")
            for external in (False, True):
                with self.subTest(external=external):
                    reader = inputs.FrozenInputs() if external else inputs.BoundedArtifacts(root)
                    references = [dict(path=str(root / name) if external else name,
                        sha256=old.sha(data)) for name, data in (("a", b"ab"), ("b", b"c"))]
                    with patch.object(inputs, "MAX_FILES", 1):
                        reader.read(references[0])
                        reader.read(references[0])  # Same immutable file adds no unique unit.
                        with patch.object(capture, "external", wraps=capture.external) as source_read, \
                                patch.object(old, "read_bounded", wraps=old.read_bounded) as retained_read, \
                                patch.object(old, "sha", wraps=old.sha) as digest:
                            with self.assertRaisesRegex(ValueError, "count bound"):
                                reader.read(references[1])
                            source_read.assert_not_called()
                            retained_read.assert_not_called()
                            digest.assert_not_called()

    def test_remaining_original_and_retained_bytes_bound_the_reader_before_hash(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "a").write_bytes(b"ab")
            (root / "b").write_bytes(b"c")
            for external in (False, True):
                with self.subTest(external=external):
                    reader = inputs.FrozenInputs() if external else inputs.BoundedArtifacts(root)
                    references = [dict(path=str(root / name) if external else name,
                        sha256=old.sha(data)) for name, data in (("a", b"ab"), ("b", b"c"))]
                    with patch.object(inputs, "MAX_BYTES", 2):
                        self.assertEqual(reader.read(references[0]), b"ab")
                        self.assertEqual(reader.read(references[0]), b"ab")
                        self.assertEqual(reader.total, 2)
                        target = capture if external else old
                        with patch.object(target, "read_bounded", wraps=target.read_bounded) as read, \
                                patch.object(target, "sha", wraps=target.sha) as digest:
                            with self.assertRaises(ValueError): reader.read(references[1])
                            self.assertEqual(read.call_args.args[1], 0)
                            digest.assert_not_called()
                    self.assertEqual(reader.total, 2)

    def test_known_file_growth_is_rejected_by_original_length_before_hashing(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "input"
            path.write_bytes(b"ab")
            reader = inputs.FrozenInputs()
            reference = dict(path=str(path), sha256=old.sha(b"ab"))
            reader.read(reference)
            path.write_bytes(b"ab-more")
            with patch.object(capture, "sha", wraps=capture.sha) as digest:
                with self.assertRaises(ValueError): reader.recheck()
                digest.assert_not_called()

    def test_original_executable_symlink_invocation_path_is_rechecked(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "first").write_bytes(b"ab")
            (root / "second").write_bytes(b"cd")
            link = root / "launcher"
            link.symlink_to(root / "first")
            reader = inputs.FrozenInputs()
            reader.read(dict(path=str(link), sha256=old.sha(b"ab")))
            self.assertIn(link, reader.seen)
            link.unlink()
            link.symlink_to(root / "second")
            with self.assertRaises(ValueError): reader.recheck()

    def test_dependency_exact_byte_capacity_and_duplicate_references_are_accepted(self):
        fixture = Fixture(self, dependency_payloads=(b"abc", b"def"))
        artifacts = inputs.BoundedArtifacts(fixture.bundle.root)
        registration = artifacts.document(fixture.registration)
        with patch.object(inputs, "MAX_DEP_BYTES", 6), patch.object(inputs, "MAX_DEP_FILES", 2):
            originals = inputs.verify_native_inputs(fixture.manifest, registration, fixture.run, artifacts)
            originals.recheck()
            artifacts.recheck()

    def test_dependency_count_or_byte_excess_stops_before_hash_and_retained_copy(self):
        for counter in ("count", "bytes"):
            with self.subTest(counter=counter):
                payload = b"def" if counter == "count" else b"defg"
                fixture = Fixture(self, dependency_payloads=(b"abc", payload))
                artifacts = inputs.BoundedArtifacts(fixture.bundle.root)
                registration = artifacts.document(fixture.registration)
                second = fixture.run["tools"][0]["dependencies"][1]
                limits = dict(MAX_DEP_FILES=1) if counter == "count" else dict(MAX_DEP_BYTES=6)
                with patch.multiple(inputs, **limits), \
                        patch.object(capture, "read_bounded", wraps=capture.read_bounded) as read, \
                        patch.object(capture, "sha", wraps=capture.sha) as digest, \
                        patch.object(artifacts, "read", wraps=artifacts.read) as retained_read:
                    with self.assertRaises(ValueError):
                        inputs.verify_native_inputs(fixture.manifest, registration, fixture.run, artifacts)
                    reads = [call for call in read.call_args_list if call.args[0] == Path(second["original"]["path"])]
                    self.assertEqual(len(reads), 0 if counter == "count" else 1)
                    if counter == "bytes": self.assertEqual(reads[0].args[1], 3)
                    self.assertFalse(any(call.args[0] == payload for call in digest.call_args_list))
                    self.assertFalse(any(call.args[0] == second["artifact"] for call in retained_read.call_args_list))


if __name__ == "__main__": unittest.main()
