"""Mocked CLI capture contracts; these fixtures are not held-out measurements."""
import copy
from contextlib import redirect_stderr, redirect_stdout
import io
import json
from pathlib import Path
import platform
from types import SimpleNamespace
import tempfile
import unittest

from benchmarks.agent_query.broad_query_capture import (
    INPUT_SCHEMA, SCHEMA, Bundle, collect, command, external, normalize, rss,
)
from benchmarks.agent_query.broad_query_review import GraphIndex, _json, main as review_main, sha
from benchmarks.agent_query.tests.test_broad_query_review import graph, registration
from benchmarks.agent_query.tests.test_broad_query_exposure import EVIDENCE, with_lineage


class PrintedEvidenceTests(unittest.TestCase):
    def test_helper_navigation_and_preview_text_do_not_supply_behavior_claims(self):
        raw = (b"Call-linked lookups: 2/40; indexed recall probes: 17/291.\n"
               b"1. Store::save @ store.py:2 [relevance=low]\n"
               b"2. Store::flush @ store.py:4 [relevance=low]\n"
               b"   Call path (execution unverified): save -> flush; navigation hint publish -> flush\n"
               b"   Helper code preview (behavior unverified) @ other.py:99: return value\n")
        result = normalize(raw, "compass", GraphIndex(graph(), "compass"), task="synthetic", arm="normalized")
        self.assertEqual(result["rankingKind"], "retrieval")
        self.assertEqual(len(result["ranked"]), 2)
        self.assertEqual(len(result["claims"]), 2)
        self.assertTrue(all(c["assertionKind"] == "declaration_identity_only" for c in result["claims"]))
        self.assertFalse(result["normalizationComplete"])
        self.assertEqual(len(result["reviewQueue"]), 5)
        self.assertEqual(sum(entry["kind"] == "unparsed_printed_text" for entry in result["reviewQueue"]), 3)

    def test_compact_seed_citations_are_printed_declarations_not_behavior(self):
        for confidence in ("exact", "approximate", "auto-picked"):
            raw = f"SEED Store::save [{confidence}] @ store.py:2:1-3:28\n".encode()
            result = normalize(raw, "compass", GraphIndex(graph(), "compass"), task="sample", arm="default")
            self.assertEqual(len(result["ranked"]), 1)
            self.assertEqual(result["rankingKind"], "display")
            self.assertEqual(result["claims"][0]["assertionKind"], "declaration_identity_only")
            self.assertEqual(len(result["claims"]), 1)
            span = result["ranked"][0]["span"]
            self.assertEqual(raw[span["startByte"]:span["endByte"]], raw.rstrip(b"\n"))
            self.assertFalse(result["normalizationComplete"])

    def test_repeated_seed_and_node_citations_do_not_create_endpoint_ambiguity(self):
        raw = (b"SEED Store::save [approximate] @ store.py:2:1-3:28\n"
               b"NODE Store::save [function] store.py:2:1-3:28\n"
               b"NODE Store::flush [function] store.py:4:1-5:12\n"
               b"EDGE Store::save --calls--> Store::flush [exact]\n")
        result = normalize(raw, "compass", GraphIndex(graph(), "compass"), task="sample", arm="default")
        self.assertEqual(len(result["ranked"]), 2)
        declarations = [item for item in result["reviewQueue"] if item["kind"] == "printed_declaration"]
        self.assertEqual(len(declarations), 3)
        self.assertEqual(declarations[0]["resolvedIdentity"], declarations[1]["resolvedIdentity"])
        relations = [item for item in result["claims"] if "relation" in item]
        self.assertEqual(len(relations), 1)
        self.assertEqual(relations[0]["relation"]["occurrences"], [])

    def test_seed_locations_and_owners_are_never_invented(self):
        for line in (
            "SEED Store::save [exact]",
            "SEED Store::save [certain] @ store.py:2:1-3:28",
            "SEED Store::save [exact] @ ../store.py:2:1-3:28",
            "SEED .save() [exact] @ store.py:2:1-3:28",
            "SEED Store::save [exact] @ store.py:99:1-99:28",
        ):
            with self.subTest(line=line):
                result = normalize((line + "\n").encode(), "compass", GraphIndex(graph(), "compass"), task="sample", arm="default")
                self.assertEqual(result["ranked"], [])
                self.assertEqual(result["claims"], [])
                self.assertTrue(result["reviewQueue"])

    def test_conflicting_or_unresolved_repeated_citations_keep_endpoint_ambiguity(self):
        for resolved in (False, True):
            fixture = graph()
            if resolved:
                other = copy.deepcopy(fixture["nodes"][0])
                other["id"] = "other-save"
                other["source"]["startLine"] = 6
                fixture["nodes"].append(other)
            raw = (b"SEED Store::save [exact] @ store.py:2:1-3:28\n"
                   b"NODE Store::save [function] store.py:6:1-7:28\n"
                   b"NODE Store::flush [function] store.py:4:1-5:12\n"
                   b"EDGE Store::save --calls--> Store::flush [exact]\n")
            result = normalize(raw, "compass", GraphIndex(fixture, "compass"), task="sample", arm="default")
            self.assertFalse(any("relation" in item for item in result["claims"]))
            declarations = [item for item in result["reviewQueue"] if item["kind"] == "printed_declaration"]
            self.assertEqual(len(declarations), 3)
            self.assertEqual("resolvedIdentity" in declarations[1], resolved)

    @staticmethod
    def relation_output(tool, edges, *, target="Store::flush"):
        if tool == "compass":
            nodes = ("NODE Store::save [function] store.py:2:1-3:28\n"
                     f"NODE {target} [function] store.py:4:1-5:12\n")
            line = lambda source, kind, target, site: f"EDGE {source} --{kind}--> {target} [exact]{site}\n"
        else:
            nodes = ("NODE Store::save [src=store.py loc=L2 community=0]\n"
                     f"NODE {target} [src=store.py loc=L4 community=0]\n")
            line = lambda source, kind, target, site: f"EDGE {source} --{kind} [EXTRACTED]--> {target}{site}\n"
        return (nodes + "".join(line(*edge) for edge in edges)).encode()

    def test_actual_printed_edge_site_range_is_queued_without_inventing_source_text(self):
        for tool in ("compass", "graphify"):
            with self.subTest(tool=tool):
                raw = self.relation_output(tool, [("Store::save", "calls", "Store::flush", " at=store.py:L3-L4")])
                result = normalize(raw, tool, GraphIndex(graph(tool), tool), task="sample", arm="default")
                relation = [c for c in result["claims"] if "relation" in c]
                self.assertEqual(len(relation), 1)
                self.assertEqual(relation[0]["relation"]["occurrences"], [])
                queued = [q for q in result["reviewQueue"] if q["kind"] == "printed_relation"][0]
                self.assertEqual(queued["targetLabel"], "Store::flush")
                self.assertEqual(queued["printedSite"], dict(file="store.py", startLine=3, endLine=4))
                self.assertNotIn("text", queued["printedSite"])
                span = queued["span"]
                self.assertEqual(raw[span["startByte"]:span["endByte"]].decode(), queued["text"])

    def test_parallel_printed_lines_keep_every_site_and_span_with_one_connectivity_claim(self):
        sites = [" at=store.py:L3", " at=store.py:L3", " at=store.py:L5", ""]
        for tool in ("compass", "graphify"):
            with self.subTest(tool=tool):
                raw = self.relation_output(tool, [("Store::save", "calls", "Store::flush", site) for site in sites])
                result = normalize(raw, tool, GraphIndex(graph(tool), tool), task="sample", arm="default")
                claims = [c for c in result["claims"] if "relation" in c]
                queued = [q for q in result["reviewQueue"] if q["kind"] == "printed_relation"]
                self.assertEqual(len(claims), 1)
                self.assertEqual(claims[0]["relation"]["occurrences"], [])
                self.assertEqual(len(queued), 4)
                self.assertEqual([q["printedSite"]["startLine"] if q["printedSite"] else None for q in queued], [3, 3, 5, None])
                self.assertEqual([q["coalescedConnectivityClaim"] for q in queued], [False, True, True, True])
                self.assertEqual({q["connectivityClaimId"] for q in queued}, {claims[0]["id"]})
                self.assertEqual(len({q["span"]["startByte"] for q in queued}), 4)
                for q in queued:
                    span = q["span"]
                    self.assertEqual(sha(raw[span["startByte"]:span["endByte"]]), span["sha256"])

    def test_different_relation_or_direction_is_not_coalesced(self):
        edges = [("Store::save", "calls", "Store::flush", ""),
                 ("Store::save", "imports", "Store::flush", ""),
                 ("Store::flush", "calls", "Store::save", "")]
        for tool in ("compass", "graphify"):
            result = normalize(self.relation_output(tool, edges), tool, GraphIndex(graph(tool), tool), task="sample", arm="default")
            self.assertEqual(len([c for c in result["claims"] if "relation" in c]), 3)

    def test_malformed_printed_sites_do_not_yield_connectivity_claims(self):
        for tool in ("compass", "graphify"):
            for suffix in ("../store.py:L3", "store.py:L0", "store.py:L4-L3", "store.py:3", "store.py:L3junk"):
                with self.subTest(tool=tool, suffix=suffix):
                    raw = self.relation_output(tool, [("Store::save", "calls", "Store::flush", " at=" + suffix)])
                    result = normalize(raw, tool, GraphIndex(graph(tool), tool), task="sample", arm="default")
                    self.assertFalse(any("relation" in c for c in result["claims"]))
                    queued = [q for q in result["reviewQueue"] if q["kind"] == "printed_relation"][0]
                    self.assertEqual(queued["malformedPrintedSite"], suffix)
                    self.assertIsNone(queued["printedSite"])
                    self.assertEqual(queued["text"], raw.decode().splitlines()[-1])

    def test_printed_sites_do_not_resolve_missing_owner_or_ambiguous_graph_identity(self):
        for tool in ("compass", "graphify"):
            for mode in ("owner", "ambiguity"):
                fixture = graph(tool)
                target = ".flush()" if mode == "owner" else "Store::flush"
                if mode == "ambiguity":
                    fixture["nodes"].append(dict(fixture["nodes"][1], id="other-flush"))
                raw = self.relation_output(tool, [("Store::save", "calls", target, " at=store.py:L3")], target=target)
                result = normalize(raw, tool, GraphIndex(fixture, tool), task="sample", arm="default")
                self.assertFalse(any("relation" in c for c in result["claims"]))
                queued = [q for q in result["reviewQueue"] if q["kind"] == "printed_relation"][0]
                self.assertEqual(queued["printedSite"]["startLine"], 3)
                self.assertNotIn("connectivityClaimId", queued)

    def test_only_printed_node_and_edges_enter_answer_not_graph_neighbors(self):
        raw = (b"SEED Store::save [exact]\n"
               b"NODE Store::save [function] store.py:2:1-3:28\n")
        result = normalize(raw, "compass", GraphIndex(graph(), "compass"), task="sample", arm="default")
        self.assertEqual([x["nodeId"] for x in result["ranked"]], ["save"])
        self.assertEqual(result["rankingKind"], "display")
        self.assertIsNone(result["admission"])
        self.assertFalse(result["normalizationComplete"])
        self.assertEqual(result["claims"][0]["assertionKind"], "declaration_identity_only")
        span = result["ranked"][0]["span"]
        self.assertEqual(sha(raw[span["startByte"]:span["endByte"]]), span["sha256"])

    def test_missing_graphify_owner_stays_unresolved_even_with_unique_graph_node(self):
        result = normalize(b"NODE .save() [src=store.py loc=L2 community=0]\n", "graphify",
                           GraphIndex(graph("graphify"), "graphify"), task="sample", arm="default")
        self.assertEqual(result["ranked"], [])
        self.assertEqual(result["claims"], [])
        self.assertEqual(result["reviewQueue"][0]["graphMatches"], ["save"])
        self.assertFalse(result["reviewQueue"][0]["ownerPrinted"])
        self.assertEqual(result["disposition"], "answered")

    def test_printed_relation_never_backfills_graph_call_site(self):
        output = (b"NODE Store::save [function] store.py:2:1-3:28\n"
                  b"NODE Store::flush [function] store.py:4:1-5:12\n"
                  b"EDGE Store::save --calls--> Store::flush [exact]\n")
        result = normalize(output, "compass", GraphIndex(graph(), "compass"), task="sample", arm="normalized")
        relation = result["claims"][-1]["relation"]
        self.assertEqual(relation["occurrences"], [])
        self.assertEqual(relation["source"]["symbol"], "save")
        self.assertEqual(relation["target"]["symbol"], "flush")

    def test_ambiguous_graph_anchor_is_not_resolved_by_expected_name_or_first_candidate(self):
        fixture = graph()
        fixture["nodes"].append(dict(fixture["nodes"][0], id="save-other"))
        result = normalize(b"NODE Store::save [function] store.py:2:1-3:28\n", "compass",
                           GraphIndex(fixture, "compass"), task="sample", arm="default")
        self.assertEqual(result["ranked"], [])
        self.assertEqual(len(result["reviewQueue"][0]["graphMatches"]), 2)

    def test_ranked_format_is_explicit_and_utf8_spans_are_bytes(self):
        raw = "préface\n1. Store::save @ store.py:2 [relevance=high]\n".encode()
        result = normalize(raw, "compass", GraphIndex(graph(), "compass"), task="sample", arm="normalized")
        self.assertEqual(result["rankingKind"], "retrieval")
        span = result["ranked"][0]["span"]
        self.assertEqual(span["startByte"], len("préface\n".encode()))
        with self.assertRaises(UnicodeError):
            normalize(b"\xff", "compass", GraphIndex(graph(), "compass"), task="sample", arm="default")

    def test_rank_gaps_do_not_promote_second_result_to_first_and_unparsed_prose_remains(self):
        raw = (b"1. .unknown() @ store.py:9 [relevance=high]\n"
               b"2. Store::save @ store.py:2 [relevance=high]\n"
               b"This may assert unsupported behavior.\n")
        result = normalize(raw, "compass", GraphIndex(graph(), "compass"), task="sample", arm="normalized")
        self.assertEqual(result["rankingKind"], "display")
        self.assertEqual(len(result["ranked"]), 1)
        self.assertTrue(any(q["kind"] == "unparsed_printed_text" for q in result["reviewQueue"]))
        with self.assertRaisesRegex(ValueError, "parser bounds"):
            normalize(b"x" * 65537, "compass", GraphIndex(graph(), "compass"), task="sample", arm="default")

    def test_rss_units_and_missing_or_duplicate_measurements(self):
        self.assertEqual(rss(b" 123 maximum resident set size\n", "darwin-l"), 123)
        self.assertEqual(rss(b"Maximum resident set size (kbytes): 123\n", "gnu-v"), 123 * 1024)
        self.assertIsNone(rss(b"no RSS", "gnu-v"))
        self.assertIsNone(rss(b"1 maximum resident set size\n2 maximum resident set size\n", "darwin-l"))


class MockCaptureTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.inputs = self.root / "inputs"
        self.inputs.mkdir()
        self.source = self.root / "source"
        self.source.mkdir()
        self.source.joinpath("store.py").write_bytes(b"class Store:\n    def save(self):\n        return self.flush()\n    def flush(self):\n        pass\n")
        self.calls = []
        self.oversized = False
        self.bad_exit = False

    def put(self, name, value):
        data = value if isinstance(value, bytes) else json.dumps(value, sort_keys=True).encode()
        path = self.inputs / name
        path.write_bytes(data)
        return dict(path=str(path), sha256=sha(data))

    def test_external_executable_symlink_is_preserved_and_retarget_drift_is_rejected(self):
        original=self.inputs/'base-python';original.write_bytes(b'original interpreter')
        replacement=self.inputs/'other-python';replacement.write_bytes(b'different interpreter')
        executable=self.inputs/'venv-python';executable.symlink_to(original)
        ref=dict(path=str(executable),sha256=sha(original.read_bytes()))
        self.assertEqual(external(ref)[0],executable)
        bundle=Bundle(self.root/'copied-provenance')
        bundle.copy('executable-bytes',ref)
        executable.unlink();executable.symlink_to(replacement)
        with self.assertRaisesRegex(ValueError,'changed while collecting'):
            bundle.recheck()

    def spec(self):
        reg = registration()
        arms = [dict(id="default"), dict(id="normalized", budgetBytes=128, measure="utf8_bytes", overflow="fail")]
        binary = self.put("fake-tool", b"not executable; bounded runner is mocked")
        timer = self.put("fake-time", b"not executable")
        env_launcher = self.put("fake-env", b"not executable")
        env = self.put("environment.json", dict(files=[binary], packageVersions={"mock": "1"}))
        release = self.put("release.json", dict(version="0.0-fixture", commit="b"*40,
                          checkedAt="2026-01-01T00:00:01Z", source="https://github.com/Graphify-Labs/graphify/releases"))
        tools = [dict(id=name, version="0.0-fixture", commit="b"*40, executable=binary,
                      argvPrefix=[], environment=env, nativeOffline=True,
                      settings=dict(scopePolicy="all tracked source", inferenceProfile="native offline",
                                    clustering="native default", queryInterface="native query default"))
                 for name in ("compass", "graphify")]
        tools[1]["releaseVerification"] = release
        inventory = self.put("inventory.json", dict(files=reg["repositories"][0]["files"]))
        build = self.put("build.json", dict(argv=[binary["path"], "extract", str(self.source)],
                         exit_code=0, timed_out=False, output_limited=False, wall_ms=5,
                         stdout_bytes=5, stderr_bytes=0,
                         stdout=self.put("build.stdout", b"built"), stderr=self.put("build.stderr", b"")))
        graphs = [dict(repository="sample", tool=name, artifact=self.put(name+"-graph.json", graph(name)),
                       sourceCommit="a"*40, sourceCleanBeforeAndAfter=True,
                       sourceInventory=inventory, buildCapture=build) for name in ("compass", "graphify")]
        from benchmarks.agent_query import broad_query_capture, broad_query_review
        plan = dict(schema=SCHEMA, tools=tools, arms=arms, graphs=graphs, timeoutSeconds=5,
                    childEnvironment=dict(inherit=False, PATH="/usr/bin:/bin", GRAPHIFY_NO_AUTO_REFRESH="1",
                                          GRAPHIFY_QUERY_LOG_DISABLE="1", **{name: "{artifactRoot}/isolated/"+name.lower() for name in
                                          ("HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME", "XDG_STATE_HOME", "APPDATA", "LOCALAPPDATA", "TMPDIR")}),
                    environmentExecutable=env_launcher,
                    normalizerSha256=sha(Path(broad_query_capture.__file__).read_bytes()),
                    assessorSha256=sha(Path(broad_query_review.__file__).read_bytes()),
                    measurement=dict(executable=timer, style="darwin-l" if platform.system()=="Darwin" else "gnu-v"))
        reg.update(arms=arms, capturePlan=plan,
                   independentReview=dict(status="confirmed", reviewerType="ai", reviewer="fixture-independent-reviewer",
                                          sourceAware=True, independentOfAuthor=True, toolLabelsBlinded=True))
        return dict(schema=INPUT_SCHEMA, registration=self.put("registration.json", reg),
                    plan=plan, sourceRoots=dict(sample=str(self.source)))

    def runner(self, argv, *, cwd, timeout_seconds, stdout_path, stderr_path):
        self.calls.append(argv)
        if argv[0] == "git":
            out = b"a"*40+b"\n" if "rev-parse" in argv else b""
            err = b""
        else:
            out = b"NODE Store::save [function] store.py:2:1-3:28\n"
            if self.oversized:
                out += b"x" * 200
            err = b" 123 maximum resident set size\n" if platform.system()=="Darwin" else b"Maximum resident set size (kbytes): 123\n"
        stdout_path.parent.mkdir(parents=True, exist_ok=True)
        stdout_path.write_bytes(out)
        stderr_path.write_bytes(err)
        return SimpleNamespace(argv=argv, exit_code=7 if self.bad_exit and argv[0]!="git" else 0,
                               timed_out=False, output_limited=False, wall_ms=10,
                               stdout=out.decode(), stderr=err.decode())

    def test_end_to_end_mocked_capture_report_and_recompute_reject_qualification(self):
        spec = self.spec()
        dest = self.root / "external-evidence"
        with redirect_stdout(io.StringIO()):
            result = collect(spec, dest, runner=self.runner)
        self.assertEqual(result["observations"], 4)
        self.assertTrue((dest / "COMPLETE.json").is_file())
        with redirect_stdout(io.StringIO()):
            self.assertEqual(review_main(["--check", str(dest / "report-provisional.json")]), 1)
        queries = [argv for argv in self.calls if "query" in argv]
        self.assertEqual(len(queries), 4)
        self.assertTrue(all("where is persistent state saved" in argv for argv in queries))
        self.assertTrue(all("--view" not in argv for argv in queries))
        self.assertEqual(sum("--text-budget" in argv for argv in queries), 1)
        self.assertEqual(sum("--budget" in argv for argv in queries), 1)
        run = _json((dest / "run.json").read_bytes())
        for entry in run["graphs"]:
            original = _json((dest / entry["originalBuildCapture"]["path"]).read_bytes())
            rebased = _json((dest / entry["buildCapture"]["path"]).read_bytes())
            self.assertEqual(original["argv"], rebased["argv"])
            self.assertEqual((dest / rebased["stdout"]["path"]).read_bytes(), b"built")
            self.assertEqual(original["stdout"]["sha256"], rebased["stdout"]["sha256"])
        with self.assertRaises(FileExistsError):
            collect(spec, dest, runner=self.runner)

    def test_heldout_capture_requires_frozen_exposure_lineage_before_any_process(self):
        spec = self.spec()
        reg = _json(Path(spec["registration"]["path"]).read_bytes())
        reg.update(designation="heldout", qualificationEligible=True)
        spec["registration"] = self.put("heldout-registration.json", reg)
        with self.assertRaisesRegex(ValueError, "exposure lineage"):
            collect(spec, self.root / "unreviewed-heldout", runner=self.runner)
        self.assertEqual(self.calls, [])
        self.assertFalse((self.root / "unreviewed-heldout").exists())

    def lineage_spec(self):
        spec = self.spec()
        reg = with_lineage(_json(Path(spec["registration"]["path"]).read_bytes()))
        spec["registration"] = self.put("lineage-registration.json", reg)
        spec["exposureEvidence"] = {sha(EVIDENCE): self.put("historical-evidence.json", EVIDENCE)}
        return spec, reg

    def test_heldout_lineage_evidence_is_copied_and_reassessed_without_qualifying(self):
        spec, reg = self.lineage_spec()
        dest = self.root / "lineage-evidence"
        with redirect_stdout(io.StringIO()):
            collect(spec, dest, runner=self.runner)
        source = _json((dest / "review-input.json").read_bytes())
        ref = source["exposureEvidence"][sha(EVIDENCE)]
        self.assertEqual((dest / ref["path"]).read_bytes(), EVIDENCE)
        self.assertEqual(_json((dest / source["registration"]["path"]).read_bytes()), reg)
        report = _json((dest / "report-provisional.json").read_bytes())
        self.assertFalse(report["assessment"]["eligibleForSuperiorityClaim"])
        self.assertFalse(any("exposure" in f for f in report["assessment"]["failures"]))
        (dest / ref["path"]).write_bytes(b"changed history")
        with redirect_stderr(io.StringIO()):
            self.assertEqual(review_main(["--check", str(dest / "report-provisional.json")]), 2)

    def test_exposed_source_and_unbound_evidence_fail_before_any_process(self):
        spec, reg = self.lineage_spec()
        reg["exposureLineage"]["history"][0]["repositoryUrl"] = reg["repositories"][0]["url"]
        spec["registration"] = self.put("exposed-registration.json", reg)
        with self.assertRaisesRegex(ValueError, "previously exposed"):
            collect(spec, self.root / "exposed-capture", runner=self.runner)
        self.assertEqual(self.calls, [])
        spec, _ = self.lineage_spec()
        spec["exposureEvidence"] = {}
        with self.assertRaisesRegex(ValueError, "references differ"):
            collect(spec, self.root / "missing-evidence", runner=self.runner)
        self.assertEqual(self.calls, [])

    def test_historical_input_drift_during_capture_leaves_no_complete_marker(self):
        spec, _ = self.lineage_spec()
        historical = Path(spec["exposureEvidence"][sha(EVIDENCE)]["path"])
        def mutate(*args, **kwargs):
            result = self.runner(*args, **kwargs)
            historical.write_bytes(b"changed after preflight")
            return result
        dest = self.root / "drifting-history"
        with self.assertRaisesRegex(ValueError, "input changed while collecting"):
            collect(spec, dest, runner=mutate)
        self.assertFalse((dest / "COMPLETE.json").exists())

    def test_parallel_printed_connectivity_does_not_break_provisional_report(self):
        def parallel_runner(argv, **kwargs):
            result = self.runner(argv, **kwargs)
            if "query" in argv:
                edges = [("Store::save", "calls", "Store::flush", " at=store.py:L3"),
                         ("Store::save", "calls", "Store::flush", " at=store.py:L5")]
                raw = b"".join(PrintedEvidenceTests.relation_output(tool, edges) for tool in ("compass", "graphify"))
                kwargs["stdout_path"].write_bytes(raw)
                result.stdout = raw.decode()
            return result
        dest = self.root / "parallel-capture"
        with redirect_stdout(io.StringIO()):
            result = collect(self.spec(), dest, runner=parallel_runner)
        self.assertEqual(result["observations"], 4)
        self.assertTrue((dest / "COMPLETE.json").is_file())
        with redirect_stdout(io.StringIO()):
            self.assertEqual(review_main(["--check", str(dest / "report-provisional.json")]), 1)

    def test_overflow_and_failed_exit_kept_in_all_denominators_and_raw_not_clipped(self):
        self.oversized = True
        self.bad_exit = True
        dest = self.root / "external-evidence"
        with redirect_stdout(io.StringIO()):
            collect(self.spec(), dest, runner=self.runner)
        report = _json((dest / "report-provisional.json").read_bytes())["assessment"]
        self.assertEqual(len(report["rows"]), 4)
        for name in ("compass", "graphify"):
            cost = report["costSummaries"][name]["normalized"]
            self.assertEqual(cost["overBudget"], 1)
            self.assertEqual(cost["failures"], 1)
        self.assertGreater(len((dest / "queries/save/compass-normalized.stdout").read_bytes()), 128)

    def test_missing_or_inconsistent_raw_build_evidence_rejects_before_queries(self):
        for mode in ("missing", "bytes", "failed"):
            spec = self.spec()
            capture = _json(Path(spec["plan"]["graphs"][0]["buildCapture"]["path"]).read_bytes())
            if mode == "missing":
                capture.pop("stdout")
            elif mode == "bytes":
                capture["stdout_bytes"] = 999
            else:
                capture["exit_code"] = 7
            changed = self.put("build.json", capture)
            for entry in spec["plan"]["graphs"]:
                entry["buildCapture"] = changed
            reg = _json(Path(spec["registration"]["path"]).read_bytes())
            reg["capturePlan"] = spec["plan"]
            spec["registration"] = self.put("registration.json", reg)
            with self.assertRaises((ValueError, TypeError)):
                collect(spec, self.root / ("bad-build-" + mode), runner=self.runner)
        self.assertEqual(self.calls, [])

    def test_pending_source_review_changed_plan_digest_and_internal_destination_fail_before_queries(self):
        for change in ("pending", "plan", "digest", "internal"):
            spec = self.spec()
            if change == "pending":
                reg = _json(Path(spec["registration"]["path"]).read_bytes())
                reg["independentReview"]["status"] = "pending"
                spec["registration"] = self.put("registration.json", reg)
            elif change == "plan":
                spec["plan"]["timeoutSeconds"] += 1
            elif change == "digest":
                Path(spec["plan"]["tools"][0]["executable"]["path"]).write_bytes(b"changed")
            destination = self.source / "artifacts" if change == "internal" else self.root / ("evidence-"+change)
            with self.assertRaises(ValueError):
                collect(spec, destination, runner=self.runner)
        self.assertEqual(self.calls, [])

    def test_unfrozen_ranked_default_is_rejected_and_optin_requires_configured_label(self):
        spec = self.spec()
        spec["plan"]["arms"][0]["compassView"] = "ranked"
        reg = _json(Path(spec["registration"]["path"]).read_bytes())
        reg["capturePlan"] = spec["plan"]; reg["arms"] = spec["plan"]["arms"]
        spec["registration"] = self.put("registration.json", reg)
        with self.assertRaisesRegex(ValueError, "default native"):
            collect(spec, self.root/"evidence", runner=self.runner)
        spec["plan"]["arms"][0].pop("compassView")
        spec["plan"]["arms"][1].update(compassView="ranked", configuredComparison=True)
        argv = command(spec["plan"], spec["plan"]["tools"][0], "normalized", "same question", Path("/graph"))
        self.assertEqual(argv[-4:], ("--view", "ranked", "--text-budget", "32"))

    def test_changed_native_argv_cannot_be_reported_under_frozen_arm(self):
        spec = self.spec()
        def altered(argv, **kwargs):
            result = self.runner(argv, **kwargs)
            if "query" in argv:
                result.argv = (*argv, "--unregistered-setting")
            return result
        destination = self.root / "wrong-command"
        with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
            with self.assertRaisesRegex(ValueError, "provisional report generation"):
                collect(spec, destination, runner=altered)
        self.assertFalse((destination / "COMPLETE.json").exists())

    def test_environment_is_explicit_nonsecret_and_contained_before_execution(self):
        for change in ("inherit", "home", "secret", "refresh"):
            spec = self.spec()
            env = spec["plan"]["childEnvironment"]
            if change == "inherit":
                env["inherit"] = True
            elif change == "home":
                env["HOME"] = str(self.source)
            elif change == "secret":
                env["UNRECOGNIZED_CREDENTIAL_KEY"] = "fixture-placeholder"
            else:
                env["GRAPHIFY_NO_AUTO_REFRESH"] = "0"
            reg = _json(Path(spec["registration"]["path"]).read_bytes())
            reg["capturePlan"] = spec["plan"]
            spec["registration"] = self.put("registration.json", reg)
            with self.assertRaises(ValueError):
                collect(spec, self.root / ("env-"+change), runner=self.runner)
        self.assertEqual(self.calls, [])

    def test_unfrozen_analysis_or_parser_code_rejects_before_execution(self):
        for field in ("assessorSha256", "normalizerSha256"):
            spec = self.spec()
            spec["plan"][field] = "0" * 64
            reg = _json(Path(spec["registration"]["path"]).read_bytes())
            reg["capturePlan"] = spec["plan"]
            spec["registration"] = self.put("registration.json", reg)
            with self.assertRaisesRegex(ValueError, "code was not frozen"):
                collect(spec, self.root / field, runner=self.runner)
        self.assertEqual(self.calls, [])

    def test_dependency_drift_during_collection_leaves_no_complete_marker(self):
        spec = self.spec()
        binary = Path(spec["plan"]["tools"][0]["executable"]["path"])
        def drifting(argv, **kwargs):
            result = self.runner(argv, **kwargs)
            if "query" in argv:
                binary.write_bytes(b"changed after collection started")
            return result
        dest = self.root / "evidence-drift"
        with self.assertRaisesRegex(ValueError, "changed while collecting"):
            collect(spec, dest, runner=drifting)
        self.assertFalse((dest / "COMPLETE.json").exists())


if __name__ == "__main__":
    unittest.main()
