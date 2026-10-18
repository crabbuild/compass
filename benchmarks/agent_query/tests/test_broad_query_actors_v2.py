"""Synthetic code-byte integrity fixtures, never source/quality attestations."""
import copy
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from benchmarks.agent_query import broad_query_actors_v2 as actors
from benchmarks.agent_query.broad_query_capture import Bundle
from benchmarks.agent_query.broad_query_review import Artifacts


def source_tree(root):
    for directory in actors.ROOTS:
        (root / directory).mkdir(parents=True)
    for module in actors.ROLES.values():
        (root / module).write_text("# Synthetic actor bytes, not executed.\n")


class ProtocolActorIntegrityTests(unittest.TestCase):
    def test_deterministic_complete_source_set_includes_nonrole_helpers(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve(); source_tree(root)
            helper = root / "benchmarks/agent_query/helper.py"; helper.write_text("pass\n")
            tests = root / "benchmarks/agent_query/tests"; tests.mkdir()
            (tests / "ignored.py").write_text("# Tests are not runtime actors.\n")
            descriptor, frozen = actors.freeze_protocol(root)
            self.assertEqual((descriptor, frozen), actors.freeze_protocol(root))
            self.assertEqual(list(frozen), sorted(frozen))
            self.assertEqual(len(descriptor["files"]), 5)
            helper.write_text("pass # changed\n")
            changed, tuple2 = actors.freeze_protocol(root)
            self.assertNotEqual(frozen["protocol"], tuple2["protocol"])
            self.assertTrue(all(frozen[role] == tuple2[role] for role in actors.ROLES))
            with self.assertRaisesRegex(ValueError, "drifted"):
                actors.verify_original_protocol(root, descriptor, frozen)

    def test_file_add_remove_and_missing_required_role_reject_original_freeze(self):
        for action in ("add", "remove", "role"):
            with self.subTest(action=action), tempfile.TemporaryDirectory() as directory:
                root = Path(directory).resolve(); source_tree(root)
                helper = root / "benchmarks/agent_query/helper.py"; helper.write_text("pass\n")
                descriptor, frozen = actors.freeze_protocol(root)
                if action == "add": (helper.parent / "added.py").write_text("pass\n")
                elif action == "remove": helper.unlink()
                else: (root / actors.ROLES["normalizer"]).unlink()
                with self.assertRaises(ValueError): actors.verify_original_protocol(root, descriptor, frozen)

    def test_retained_and_original_actor_bytes_round_trip_under_shared_reader(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve() / "product"; root.mkdir(); source_tree(root)
            descriptor, frozen = actors.freeze_protocol(root)
            bundle = Bundle(Path(directory) / "evidence")
            binding = actors.retain_protocol(bundle, root, descriptor, frozen)
            reader = Artifacts(bundle.root)
            self.assertEqual(actors.verify_retained_protocol(binding, reader, root), (descriptor, frozen))
            self.assertEqual(len(reader.seen), len(descriptor["files"]) + 1)
            reader.recheck(); bundle.recheck()

    def test_malformed_mixed_and_reordered_retained_bindings_reject(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve() / "product"; root.mkdir(); source_tree(root)
            descriptor, frozen = actors.freeze_protocol(root)
            bundle = Bundle(Path(directory) / "evidence")
            binding = actors.retain_protocol(bundle, root, descriptor, frozen)
            for change in ("schema", "missing", "reorder", "actor", "artifact", "extra"):
                bad = copy.deepcopy(binding)
                if change == "schema": bad["schema"] = "compass.broad-query-protocol-binding/1"
                elif change == "missing": bad["files"].pop()
                elif change == "reorder": bad["files"].reverse()
                elif change == "actor": bad["actorSha256"]["normalizer"] = "0" * 64
                elif change == "artifact": bad["files"][0]["artifact"] = []
                else: bad["extra"] = True
                with self.subTest(change=change), self.assertRaises(ValueError):
                    actors.verify_retained_protocol(bad, Artifacts(bundle.root), root)

    def test_retained_code_drift_cannot_hide_behind_unchanged_actor_metadata(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve() / "product"; root.mkdir(); source_tree(root)
            descriptor, frozen = actors.freeze_protocol(root)
            bundle = Bundle(Path(directory) / "evidence")
            binding = actors.retain_protocol(bundle, root, descriptor, frozen)
            path = bundle.root / binding["files"][0]["artifact"]["path"]
            path.write_text("# changed retained source\n")
            with self.assertRaises(ValueError): actors.verify_retained_protocol(binding, Artifacts(bundle.root), root)

    def test_original_helper_drift_during_retained_read_is_rechecked(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve() / "product"; root.mkdir(); source_tree(root)
            helper = root / "benchmarks/agent_query/helper.py"; helper.write_text("pass\n")
            descriptor, frozen = actors.freeze_protocol(root)
            bundle = Bundle(Path(directory) / "evidence")
            binding = actors.retain_protocol(bundle, root, descriptor, frozen)
            reader = Artifacts(bundle.root); original = reader.read
            changed = False
            def read(ref, *args):
                nonlocal changed
                data = original(ref, *args)
                if not changed and ref != binding["protocolManifest"]:
                    helper.write_text("# changed during read\n"); changed = True
                return data
            with patch.object(reader, "read", side_effect=read), self.assertRaisesRegex(ValueError, "drifted"):
                actors.verify_retained_protocol(binding, reader, root)

    def test_symlinked_actor_root_and_source_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve(); source_tree(root)
            source = root / actors.ROLES["normalizer"]
            original = source.with_suffix(".txt"); source.rename(original)
            try: source.symlink_to(original)
            except OSError as error: self.skipTest(f"symlinks unavailable: {error}")
            with self.assertRaises(ValueError): actors.freeze_protocol(root)
            source.unlink(); source.write_text("pass\n")
            base = root / actors.ROOTS[1]; moved = base.with_name("moved"); base.rename(moved); base.symlink_to(moved)
            with self.assertRaises(ValueError): actors.freeze_protocol(root)

    def test_actual512_file_cap_then_first_excess_before_source_read(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve(); source_tree(root)
            folder = root / actors.ROOTS[0]
            for index in range(512 - len(actors.ROLES)):
                (folder / f"helper{index:04}.py").write_text("pass\n")
            self.assertEqual(len(actors.freeze_protocol(root)[0]["files"]), 512)
            (folder / "excess.py").write_text("pass\n")
            with patch.object(actors, "read_bounded", side_effect=AssertionError("over-bound source read")):
                with self.assertRaisesRegex(ValueError, "count exceeds512"): actors.freeze_protocol(root)

    def test_configured_byte_and_tree_boundaries_are_inclusive(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve(); source_tree(root)
            size = sum((root / module).stat().st_size for module in actors.ROLES.values())
            with patch.object(actors, "MAX_BYTES", size): actors.freeze_protocol(root)
            with patch.object(actors, "MAX_BYTES", size - 1), self.assertRaises(ValueError): actors.freeze_protocol(root)
            with patch.object(actors, "MAX_TREE_ENTRIES", len(actors.ROLES)): actors.freeze_protocol(root)
            with patch.object(actors, "MAX_TREE_ENTRIES", len(actors.ROLES) - 1), self.assertRaises(ValueError): actors.freeze_protocol(root)
            with patch.object(actors, "MAX_TREE_DIRECTORIES", 2): actors.freeze_protocol(root)
            with patch.object(actors, "MAX_TREE_DIRECTORIES", 1), self.assertRaises(ValueError): actors.freeze_protocol(root)
            (root / actors.ROOTS[0] / "extra").mkdir()
            with patch.object(actors, "MAX_TREE_DIRECTORIES", 2), self.assertRaises(ValueError): actors.freeze_protocol(root)


if __name__ == "__main__": unittest.main()
