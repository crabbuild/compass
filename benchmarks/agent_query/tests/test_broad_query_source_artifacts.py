"""Coherent native graph inputs through collection and canonical reassessment."""
import copy
from contextlib import redirect_stderr, redirect_stdout
import io
import json
from pathlib import Path
import unittest

from benchmarks.agent_query.broad_query_capture import collect
from benchmarks.agent_query.broad_query_review import _json, assess, sha
from benchmarks.agent_query.tests import test_broad_query_capture as fixture


class SourceArtifactTests(unittest.TestCase):
    setUp = fixture.MockCaptureTests.setUp
    put = fixture.MockCaptureTests.put
    spec = fixture.MockCaptureTests.spec
    runner = fixture.MockCaptureTests.runner

    def source_spec(self):
        spec = self.spec()
        entry = next(g for g in spec["plan"]["graphs"] if g["tool"] == "compass")
        sidecar = dict(schema="compass.source-query/1", profile="source-query/4-nested-callable-context",
                       graphDigest=entry["artifact"]["sha256"], files=[], owners=[],
                       aggregateTruncated=False, omittedOwners=0, omittedFiles=0,
                       additionalDocumentFrequencies={})
        entry["sourceQueryArtifact"] = self.put("source-query.json", sidecar)
        self.freeze(spec)
        return spec

    def freeze(self, spec):
        reg = _json(Path(spec["registration"]["path"]).read_bytes())
        reg["capturePlan"] = spec["plan"]
        spec["registration"] = self.put("registration.json", reg)

    def capture(self, spec, name="captured", runner=None):
        dest = self.root / name
        with redirect_stdout(io.StringIO()):
            collect(spec, dest, runner=runner or self.runner)
        return dest, _json((dest / "run.json").read_bytes())

    def test_companion_reaches_the_actual_native_query_path(self):
        spec = self.source_spec()
        expected = Path(spec["plan"]["graphs"][0]["sourceQueryArtifact"]["path"]).read_bytes()
        seen = []
        def inspect(argv, **kwargs):
            if "query" in argv and argv[argv.index("query") - 1] == spec["plan"]["tools"][0]["executable"]["path"]:
                graph = Path(argv[argv.index("--graph") + 1])
                # Both tools share a fake executable; check the isolated native layout.
                if graph.parent.name.endswith("-compass"):
                    seen.append((graph.parent / "source-query.json").read_bytes())
            return self.runner(argv, **kwargs)
        dest, run = self.capture(spec, runner=inspect)
        native = next(g for g in run["graphs"] if g["tool"] == "compass")
        self.assertIn("sourceQueryArtifact", native)
        self.assertEqual(len(seen), 2)
        self.assertEqual(seen, [expected, expected])
        manifest = _json((dest / "review-input.json").read_bytes())
        self.assertFalse(assess(manifest, dest)["eligibleForSuperiorityClaim"])

    def rewrite_run(self, dest, mutate):
        manifest = _json((dest / "review-input.json").read_bytes())
        run = _json((dest / manifest["run"]["path"]).read_bytes())
        mutate(run)
        def put(name, value):
            raw = json.dumps(value).encode()
            (dest / name).write_bytes(raw)
            return dict(path=name, sha256=sha(raw))
        manifest["run"] = put("changed-run.json", run)
        reviews = _json((dest / manifest["reviews"]["path"]).read_bytes())
        reviews["runSha256"] = manifest["run"]["sha256"]
        manifest["reviews"] = put("changed-reviews.json", reviews)
        return manifest

    def test_two_repositories_keep_distinct_fixed_sibling_artifacts(self):
        spec = self.source_spec()
        reg = _json(Path(spec["registration"]["path"]).read_bytes())
        reg["repositories"].append(dict(copy.deepcopy(reg["repositories"][0]), id="other"))
        reg["tasks"].append(dict(copy.deepcopy(reg["tasks"][0]), id="other-save", repository="other", cluster="other"))
        spec["registration"] = self.put("registration.json", reg)
        spec["sourceRoots"]["other"] = str(self.source)
        (self.inputs / "other").mkdir()
        for entry in list(spec["plan"]["graphs"]):
            other = dict(copy.deepcopy(entry), repository="other")
            graph = _json(Path(entry["artifact"]["path"]).read_bytes())
            graph["fixtureRepository"] = "other"
            other["artifact"] = self.put("other/" + entry["tool"] + ".json", graph)
            if entry["tool"] == "compass":
                sidecar = _json(Path(entry["sourceQueryArtifact"]["path"]).read_bytes())
                sidecar["graphDigest"] = other["artifact"]["sha256"]
                other["sourceQueryArtifact"] = self.put("other/source-query.json", sidecar)
            spec["plan"]["graphs"].append(other)
        self.freeze(spec)
        seen = {}
        def inspect(argv, **kwargs):
            if "--graph" in argv:
                path = Path(argv[argv.index("--graph") + 1])
                if path.parent.name.endswith("-compass"):
                    seen[path.parent.name] = _json((path.parent / "source-query.json").read_bytes())["graphDigest"]
            return self.runner(argv, **kwargs)
        dest, run = self.capture(spec, runner=inspect)
        self.assertEqual(set(seen), {"sample-compass", "other-compass"})
        self.assertEqual(len(set(seen.values())), 2)
        self.assertEqual(len(run["observations"]), 8)
        self.assertTrue((dest / "COMPLETE.json").is_file())

    def test_invalid_declared_envelopes_reject_before_any_process(self):
        for change in [dict(schema="compass.source-query/99"), dict(profile="source-query/99"),
                       dict(graphDigest="f" * 64), dict(graphDigest=None)]:
            spec = self.source_spec()
            entry = spec["plan"]["graphs"][0]
            value = _json(Path(entry["sourceQueryArtifact"]["path"]).read_bytes())
            entry["sourceQueryArtifact"] = self.put("source-query.json", dict(value, **change))
            self.freeze(spec)
            with self.subTest(change=change), self.assertRaises(ValueError):
                self.capture(spec)
            self.assertEqual(self.calls, [])

    def test_bad_references_and_symlinks_reject_before_any_process(self):
        for mode in ("null", "extra", "hash", "wrong-path", "graphify", "symlink"):
            spec = self.source_spec()
            entry = spec["plan"]["graphs"][0]
            ref = entry["sourceQueryArtifact"]
            if mode == "null":
                entry["sourceQueryArtifact"] = None
            elif mode == "extra":
                ref["other"] = True
            elif mode == "hash":
                ref["sha256"] = "f" * 64
            elif mode == "wrong-path":
                ref["path"] = str(self.inputs / "elsewhere.json")
            elif mode == "graphify":
                spec["plan"]["graphs"][1]["sourceQueryArtifact"] = ref
            else:
                path = Path(ref["path"])
                target = self.inputs / "sidecar-target.json"
                target.write_bytes(path.read_bytes())
                path.unlink()
                path.symlink_to(target)
            self.freeze(spec)
            with self.subTest(mode=mode), self.assertRaises(ValueError):
                self.capture(spec)
            self.assertEqual(self.calls, [])
            if mode == "symlink":
                path.unlink()

    def test_existing_source_evidence_cannot_be_silently_dropped(self):
        spec = self.source_spec()
        del spec["plan"]["graphs"][0]["sourceQueryArtifact"]
        self.freeze(spec)
        with self.assertRaisesRegex(ValueError, "undeclared source-query"):
            self.capture(spec)
        self.assertEqual(self.calls, [])

    def test_legacy_graph_only_layout_remains_explicitly_graph_only(self):
        dest, run = self.capture(self.spec())
        native = next(g for g in run["graphs"] if g["tool"] == "compass")
        self.assertNotIn("sourceQueryArtifact", native)
        self.assertEqual(native["artifact"]["path"], "graphs/sample-compass-artifact.json")
        manifest = _json((dest / "review-input.json").read_bytes())
        self.assertFalse(assess(manifest, dest)["eligibleForSuperiorityClaim"])
        (dest / "graphs/source-query.json").write_bytes(b"undeclared")
        with self.assertRaisesRegex(ValueError, "undeclared source-query"):
            assess(manifest, dest)

    def test_source_or_retained_mutation_prevents_completion(self):
        for mode in ("input", "retained", "retained-symlink"):
            spec = self.source_spec()
            changed = False
            def mutate(argv, **kwargs):
                nonlocal changed
                result = self.runner(argv, **kwargs)
                if "--graph" in argv and not changed:
                    graph = Path(argv[argv.index("--graph") + 1])
                    if graph.parent.name.endswith("-compass"):
                        path = (Path(spec["plan"]["graphs"][0]["sourceQueryArtifact"]["path"])
                                if mode == "input" else graph.parent / "source-query.json")
                        if mode == "retained-symlink":
                            target = graph.parent / "same-bytes.json"
                            target.write_bytes(path.read_bytes())
                            path.unlink()
                            path.symlink_to(target)
                        else:
                            path.write_bytes(b"changed")
                        changed = True
                return result
            dest = self.root / mode
            with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()), self.assertRaises(ValueError):
                collect(spec, dest, runner=mutate)
            self.assertTrue(changed)
            self.assertFalse((dest / "COMPLETE.json").exists())

    def test_a_rehashed_retained_sidecar_cannot_replace_the_frozen_input(self):
        dest, run = self.capture(self.source_spec())
        native = next(g for g in run["graphs"] if g["tool"] == "compass")
        path = dest / native["sourceQueryArtifact"]["path"]
        value = _json(path.read_bytes())
        value["omittedFiles"] = 1
        raw = json.dumps(value).encode()
        path.write_bytes(raw)
        def mutate(run):
            next(g for g in run["graphs"] if g["tool"] == "compass")["sourceQueryArtifact"]["sha256"] = sha(raw)
        manifest = self.rewrite_run(dest, mutate)
        with self.assertRaisesRegex(ValueError, "source-query artifact differs from source-frozen"):
            assess(manifest, dest)

    def test_retained_layout_cannot_redirect_native_auxiliary_input(self):
        for mode in ("omitted", "redirected", "symlink"):
            dest, run = self.capture(self.source_spec(), name=mode)
            native = next(g for g in run["graphs"] if g["tool"] == "compass")
            path = dest / native["sourceQueryArtifact"]["path"]
            if mode == "symlink":
                other = path.with_name("other.json")
                other.write_bytes(path.read_bytes())
                path.unlink()
                path.symlink_to(other)
            def mutate(run):
                entry = next(g for g in run["graphs"] if g["tool"] == "compass")
                if mode == "omitted":
                    del entry["sourceQueryArtifact"]
                elif mode == "redirected":
                    entry["sourceQueryArtifact"]["path"] = "other.json"
            manifest = self.rewrite_run(dest, mutate)
            with self.subTest(mode=mode), self.assertRaises(ValueError):
                assess(manifest, dest)

    def test_final_recheck_catches_undeclared_or_nonregular_inputs(self):
        from benchmarks.agent_query.broad_query_review import Artifacts
        dest, run = self.capture(self.source_spec())
        native = next(g for g in run["graphs"] if g["tool"] == "compass")
        audit = Artifacts(dest)
        audit.source_query(native)
        path = dest / native["sourceQueryArtifact"]["path"]
        other = path.with_name("other.json")
        other.write_bytes(path.read_bytes())
        path.unlink()
        path.symlink_to(other)
        with self.assertRaisesRegex(ValueError, "regular file"):
            audit.recheck()
        path.unlink()
        graph_only = dict(native)
        del graph_only["sourceQueryArtifact"]
        audit = Artifacts(dest)
        audit.source_query(graph_only)
        path.write_bytes(other.read_bytes())
        with self.assertRaisesRegex(ValueError, "undeclared source-query"):
            audit.recheck()

    def test_native_envelope_constants_remain_synchronized(self):
        from benchmarks.agent_query import broad_query_review as review
        source = Path(__file__).resolve().parents[3] / "crates/compass-model/src/source_query.rs"
        raw = source.read_text()
        for profile in review.SOURCE_QUERY_PROFILES:
            self.assertIn('= "' + profile + '";', raw)
        self.assertIn('SOURCE_QUERY_FILE: &str = "' + review.SOURCE_QUERY_FILE + '";', raw)
        self.assertIn("SOURCE_QUERY_MAX_BYTES: u64 = 64 * 1024 * 1024;", raw)
        self.assertEqual(review.MAX_SOURCE_QUERY_BYTES, 64 * 1024 * 1024)
