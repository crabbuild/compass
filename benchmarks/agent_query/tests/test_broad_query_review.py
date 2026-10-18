"""Synthetic scorer/qualification-contract tests, not independent source labels."""
import copy
from contextlib import redirect_stderr, redirect_stdout
import io
from pathlib import Path
import json
import tempfile
import unittest

from benchmarks.agent_query.broad_query_review import (
    ARMS, MANIFEST_SCHEMA, NORMALIZED_SCHEMA, REGISTRATION_SCHEMA, REPORT_SCHEMA,
    REVIEW_SCHEMA, RUN_SCHEMA, Artifacts, GraphIndex, Identity, _json, _review_failures,
    assess, main,
    finite_ni_power, paired_interval, score_task, sha, validate_registration,
    _verify_sources,
)


def identity(symbol="save", line=2, owner="Store", file="store.py"):
    return dict(file=file, startLine=line, symbol=symbol, owner=owner)


def site(line=3, text="return self.flush()"):
    return dict(file="store.py", line=line, text=text)


def relation():
    return dict(source=identity(), target=identity("flush", 4), kind="calls",
                direction="source_to_target", occurrences=[site()])


def task(kind="broad"):
    return dict(id="save", repository="sample", cluster="save-mechanism", kind=kind,
                question="where is persistent state saved", expected="answer",
                relevant=[dict(identity=identity(), grade=3)],
                facts=[dict(id="saves", identity=identity(), essential=True,
                            source=site(), statement="save calls flush")])


def response():
    return dict(disposition="answered", rankingKind="retrieval", admission=None,
                ranked=[dict(identity=identity(), nodeId="save", firstPage=True)],
                claims=[dict(id="claim-1", identity=identity())])


def judgment():
    return dict(candidates=[], claims=[dict(id="claim-1", factId="saves", verdict="supported",
                                          reason="Reviewed the enclosing declaration and source call.")])


def graph(tool="compass", *, directed=True):
    nodes = []
    for node_id, name, line in [("save", "save", 2), ("flush", "flush", 4)]:
        if tool == "compass":
            nodes.append(dict(id=node_id, name=name, qualifiedName="Store::" + name,
                              source=dict(file="store.py", startLine=line)))
        else:
            nodes.append(dict(id=node_id, label="." + name + "()", source_file="store.py",
                              source_location=f"L{line}"))
    edge = dict(source="save", target="flush")
    if tool == "compass":
        edge.update(kind="calls", relationshipSite=dict(file="store.py", startLine=3))
    else:
        edge.update(relation="calls", source_file="store.py", source_location="L3")
    return dict(directed=directed, multigraph=tool == "compass", nodes=nodes, edges=[edge])


def registration():
    source = b"class Store:\n    def save(self):\n        return self.flush()\n    def flush(self):\n        pass\n"
    return dict(schema=REGISTRATION_SCHEMA, designation="development", frozenAt="2026-01-01T00:00:00Z",
                qualificationEligible=False, sourceReview=dict(author="AI source author", blindToOutputs=True),
                independentReview=dict(status="pending", reviewer=None, toolLabelsBlinded=True),
                repositories=[dict(id="sample", language="python", url="https://example.invalid/sample",
                                   commit="a" * 40, files=[dict(file="store.py", sha256=sha(source))])],
                tasks=[task()])


class SourceFactScoringTests(unittest.TestCase):
    def test_typed_source_fact_succeeds_without_expected_prose_substrings(self):
        scored = score_task(task(), response(), judgment())
        self.assertTrue(scored["success"])
        self.assertEqual(scored["successAt5"], True)
        self.assertEqual(scored["sourceFactRecall"], 1)
        self.assertEqual(scored["claimPrecision"], 1)

    def test_right_file_wrong_owner_is_not_a_correct_answer(self):
        for changed in [identity(owner="UnrelatedStore"), identity(line=1), identity(symbol="save_backup")]:
            answer = response()
            answer["ranked"][0]["identity"] = changed
            answer["claims"][0]["identity"] = changed
            scored = score_task(task(), answer, judgment())
            self.assertFalse(scored["success"])
            self.assertEqual(scored["unsupportedClaims"], 1)

    def test_names_without_source_fact_answer_do_not_pass(self):
        answer = response()
        answer["claims"] = []
        review = judgment()
        review["claims"] = []
        scored = score_task(task(), answer, review)
        self.assertEqual(scored["successAt5"], True)
        self.assertFalse(scored["success"])
        self.assertEqual(scored["sourceFactRecall"], 0)

    def test_ambiguity_list_cannot_pass_an_answer_question(self):
        answer = response()
        answer["disposition"] = "ambiguous"
        self.assertFalse(score_task(task(), answer, judgment())["success"])

    def test_relation_direction_and_occurrence_are_exact(self):
        expected = task("directed")
        expected["facts"][0]["relation"] = relation()
        answer = response()
        answer["claims"][0]["relation"] = relation()
        self.assertTrue(score_task(expected, answer, judgment())["success"])
        for mutate in ["reverse", "wrong-site", "wrong-relation"]:
            wrong = copy.deepcopy(answer)
            edge = wrong["claims"][0]["relation"]
            if mutate == "reverse":
                edge["source"], edge["target"] = edge["target"], edge["source"]
            elif mutate == "wrong-site":
                edge["occurrences"][0]["line"] = 4
            else:
                edge["kind"] = "references"
            self.assertFalse(score_task(expected, wrong, judgment())["success"])

    def test_connectivity_rubric_allows_absent_sites_but_never_wrong_printed_sites(self):
        expected = task("directed")
        fact = expected["facts"][0]
        fact.update(relation=relation(), requireOccurrences=False)
        answer = response()
        answer["claims"][0]["relation"] = dict(relation(), occurrences=[])
        self.assertTrue(score_task(expected, answer, judgment())["success"])
        self.assertEqual(answer["claims"][0]["relation"]["occurrences"], [])
        fact["requireOccurrences"] = True
        self.assertFalse(score_task(expected, answer, judgment())["success"])
        fact["requireOccurrences"] = False
        answer["claims"][0]["relation"]["occurrences"] = [site(999, "other.flush()")]
        self.assertFalse(score_task(expected, answer, judgment())["success"])

    def test_repeated_source_sites_are_not_silently_covered_by_one_site(self):
        expected = task("directed")
        expected["facts"][0]["relation"] = relation()
        expected["facts"][0]["relation"]["occurrences"].append(site(5, "self.flush()"))
        answer = response()
        answer["claims"][0]["relation"] = relation()
        self.assertFalse(score_task(expected, answer, judgment())["success"])

    def test_no_answer_is_correct_only_for_registered_unsupported_task(self):
        answer = dict(disposition="abstain", rankingKind="unavailable", admission=None, ranked=[], claims=[])
        review = dict(claims=[], candidates=[])
        scored = score_task(task(), answer, review)
        self.assertFalse(scored["success"])
        self.assertTrue(scored["falseEmpty"])
        unsupported = dict(task(), kind="unsupported", expected="abstain", relevant=[], facts=[],
                           rationale="No source-supported implementation in the registered inventory.")
        self.assertTrue(score_task(unsupported, answer, review)["success"])
        unexpected = dict(claims=[dict(id="claim-1", factId=None, verdict="unsupported", reason="No matching source fact.")],
                          candidates=[dict(identity=identity(), grade=0)])
        self.assertFalse(score_task(unsupported, response(), unexpected)["success"])
        answer["ranked"] = response()["ranked"]
        answer["rankingKind"] = "display"
        with self.assertRaisesRegex(ValueError, "abstention"):
            score_task(unsupported, answer, review)

    def test_legitimate_ambiguity_needs_two_reviewed_distinct_identities(self):
        expected = task("ambiguous")
        expected.update(expected="ambiguous", facts=[])
        expected["relevant"].append(dict(identity=identity("flush", 4), grade=3))
        answer = response()
        answer.update(disposition="ambiguous", claims=[])
        review = dict(claims=[], candidates=[])
        self.assertFalse(score_task(expected, answer, review)["success"])
        answer["ranked"].append(dict(identity=identity("flush", 4), nodeId="flush", firstPage=True))
        self.assertTrue(score_task(expected, answer, review)["success"])

    def test_unknown_candidates_and_unreviewed_claims_prevent_qualification(self):
        answer = response()
        answer["ranked"].append(dict(identity=identity("flush", 4), nodeId="flush", firstPage=True))
        scored = score_task(task(), answer, judgment())
        self.assertFalse(scored["judgmentComplete"])
        self.assertFalse(scored["success"])
        review = judgment()
        review["candidates"] = [dict(identity=identity("flush", 4), grade=0)]
        self.assertTrue(score_task(task(), answer, review)["success"])
        self.assertFalse(score_task(task(), response(), dict(claims=[], candidates=[]))["success"])

    def test_post_output_oracle_correction_is_not_a_new_heldout_success(self):
        answer = response()
        review = judgment()
        review["candidates"] = [dict(identity=identity("flush", 4), grade=3)]
        scored = score_task(task(), answer, review)
        self.assertFalse(scored["judgmentComplete"])
        self.assertEqual(scored["postOutputPositiveCount"], 1)
        review["candidates"] = [dict(identity=identity(), grade=0)]
        with self.assertRaisesRegex(ValueError, "silently changes"):
            score_task(task(), answer, review)

    def test_display_order_is_not_retrieval_rank(self):
        answer = response()
        answer["rankingKind"] = "display"
        scored = score_task(task(), answer, judgment())
        self.assertTrue(scored["success"])
        self.assertIsNone(scored["mrrAt10"])
        self.assertIsNone(scored["ndcgAt10"])

    def test_relevant_rank_six_is_not_success_at_five(self):
        answer = response()
        answer["ranked"] = [dict(identity=identity(f"other-{n}", n + 8), nodeId=f"other-{n}", firstPage=False)
                            for n in range(5)] + answer["ranked"]
        scored = score_task(task(), answer, judgment())
        self.assertFalse(scored["successAt5"])
        self.assertAlmostEqual(scored["mrrAt10"], 1 / 6)
        self.assertAlmostEqual(scored["ndcgAt10"], 1 /  __import__('math').log2(7))

    def test_unsupported_extra_assertion_cannot_hide_behind_a_correct_fact(self):
        answer = response()
        answer["claims"].append(dict(id="invented", identity=identity("flush", 4)))
        review = judgment()
        review["claims"].append(dict(id="invented", verdict="unsupported", factId=None,
                                     reason="Source does not implement this behavior."))
        scored = score_task(task(), answer, review)
        self.assertFalse(scored["success"])
        self.assertEqual(scored["claimPrecision"], .5)

    def test_duplicate_facts_and_duplicate_candidates_do_not_inflate_metrics(self):
        answer = response()
        answer["ranked"] *= 2
        with self.assertRaisesRegex(ValueError, "duplicate"):
            score_task(task(), answer, judgment())
        answer = response()
        answer["claims"].append(dict(id="repeated", identity=identity()))
        review = judgment()
        review["claims"].append(dict(review["claims"][0], id="repeated"))
        with self.assertRaisesRegex(ValueError, "semantic claims"):
            score_task(task(), answer, review)


class GraphAndLedgerTests(unittest.TestCase):
    def test_exact_identity_and_graph_direction_are_separate(self):
        for tool in ("compass", "graphify"):
            index = GraphIndex(graph(tool), tool)
            self.assertEqual(index.resolve(Identity.parse(identity())), ["save"])
            expected = dict(task()["facts"][0], relation=relation())
            self.assertEqual(index.fact_status(expected), "present")
        index = GraphIndex(graph("graphify", directed=False), "graphify")
        self.assertEqual(index.fact_status(expected), "unavailable_direction_contract")

    def test_missing_ambiguous_wrong_site_reverse_and_duplicate_nodes_fail_closed(self):
        fact = dict(task()["facts"][0], relation=relation())
        for mutate in ("reverse", "wrong-site"):
            raw = graph()
            if mutate == "reverse":
                raw["edges"][0]["source"], raw["edges"][0]["target"] = "flush", "save"
            else:
                raw["edges"][0]["relationshipSite"]["startLine"] = 5
            self.assertEqual(GraphIndex(raw, "compass").fact_status(fact), "missing")
        raw = graph()
        raw["nodes"].append(dict(raw["nodes"][0], id="second-save"))
        index = GraphIndex(raw, "compass")
        self.assertEqual(index.fact_status(fact), "ambiguous")
        with self.assertRaisesRegex(ValueError, "uniquely resolve"):
            score_task(task(), response(), judgment(), index)
        raw["nodes"][-1]["id"] = "save"
        with self.assertRaisesRegex(ValueError, "duplicate"):
            GraphIndex(raw, "compass")

    def test_ledger_does_not_invent_admission_instrumentation(self):
        scored = score_task(task(), response(), judgment(), GraphIndex(graph(), "compass"))
        ledger = scored["ledger"][0]
        self.assertEqual(ledger["graph"], "present")
        self.assertEqual(ledger["admission"], "not_instrumented")
        self.assertEqual(ledger["rank"], 1)
        self.assertTrue(ledger["firstPage"])
        answer = dict(response(), admission=["save"])
        self.assertEqual(score_task(task(), answer, judgment(), GraphIndex(graph(), "compass"))["ledger"][0]["admission"], "admitted")
        answer["admission"] = []
        with self.assertRaisesRegex(ValueError, "admission ledger"):
            score_task(task(), answer, judgment(), GraphIndex(graph(), "compass"))

    def test_captured_span_must_match_exact_bytes(self):
        answer = response()
        output = b"save the state"
        span = dict(startByte=0, endByte=4, sha256=sha(b"save"))
        answer["ranked"][0]["span"] = span
        answer["claims"][0]["span"] = span
        self.assertTrue(score_task(task(), answer, judgment(), GraphIndex(graph(), "compass"), output)["success"])
        answer["claims"][0]["span"] = dict(span, sha256="0" * 64)
        with self.assertRaisesRegex(ValueError, "span"):
            score_task(task(), answer, judgment(), GraphIndex(graph(), "compass"), output)


class UncertaintyAndRegistrationTests(unittest.TestCase):
    def test_paired_interval_is_order_invariant_and_clusters_paraphrases(self):
        rows = [(f"r{r}", f"task{t}", float((r+t) % 3 != 0), float(t % 2))
                for r in range(8) for t in range(10)]
        first = paired_interval(rows, samples=1000)
        self.assertEqual(first, paired_interval(list(reversed(rows)), samples=1000))
        duplicated = paired_interval(rows + rows, samples=1000)
        self.assertEqual(first, duplicated)
        self.assertEqual(first["clusters"], 80)
        self.assertEqual(first["repositories"], 8)
        self.assertLess(first["lower"], first["upper"])

    def test_small_or_degenerate_panel_does_not_fabricate_uncertainty(self):
        result = paired_interval([("one", "one", 1, 0)], samples=1000)
        self.assertTrue(result["degenerate"])
        self.assertEqual(result["repositories"], 1)
        self.assertLess(result["lower"], 0)
        self.assertIsNone(paired_interval([], samples=1000)["estimate"])

    def test_all_tie_controls_need_finite_sample_evidence_before_two_point_ni(self):
        small = paired_interval([(f"r{r}", f"c{n}", 1, 1) for r in range(8) for n in range(4)], samples=1000)
        self.assertEqual(small["bootstrapLower"], 0)
        self.assertLess(small["lower"], -.02)
        self.assertEqual(small["clusters"], 32)
        larger = paired_interval([(f"r{r}", f"c{n}", 1, 1) for r in range(8) for n in range(32)], samples=1000)
        self.assertGreater(larger["lower"], -.02)
        self.assertIn("fixed repository panel", larger["finiteRiskScope"])

    def test_exact_ni_power_not_normal_approximation_or_all_tie_posthoc_power(self):
        small = finite_ni_power(32, 0)
        self.assertEqual(small["maximumAdverseClusters"], -1)
        self.assertEqual(small["finiteBoundPassProbability"], 0)
        equality = finite_ni_power(192, 0)
        self.assertEqual(equality["finiteBoundPassProbability"], 1)
        realistic = finite_ni_power(200, .005)
        self.assertAlmostEqual(realistic["finiteBoundPassProbability"], .995 ** 200)
        self.assertLess(realistic["finiteBoundPassProbability"], .8)
        self.assertGreater(finite_ni_power(600, .005)["finiteBoundPassProbability"], .9)

    def test_review_type_and_independence_are_explicit_not_assumed(self):
        for kind in ("human", "ai"):
            review = dict(status="confirmed", reviewerType=kind, reviewer="second reviewer",
                          sourceAware=True, independentOfAuthor=True, toolLabelsBlinded=True)
            self.assertEqual(_review_failures(review, "fixture"), [])
            review["independentOfAuthor"] = False
            self.assertTrue(_review_failures(review, "fixture"))

    def test_pending_ai_authored_registration_is_valid_but_not_qualified(self):
        registered = validate_registration(registration())
        self.assertFalse(registered["qualificationEligible"])
        for field in [dict(file="../secret", startLine=2, symbol="save", owner="Store"),
                      dict(file="store.py", startLine=True, symbol="save", owner="Store")]:
            wrong = registration()
            wrong["tasks"][0]["relevant"][0]["identity"] = field
            with self.assertRaises(ValueError):
                validate_registration(wrong)

    def test_tab_indented_source_witnesses_round_trip_and_match_pinned_lines(self):
        registered = registration()
        source = b"class Store:\n\tdef save(self):\n\t\treturn self.flush()\n\tdef flush(self):\n\t\tpass\n"
        registered["repositories"][0]["files"][0]["sha256"] = sha(source)
        fact = registered["tasks"][0]["facts"][0]
        fact["source"] = site(text="\t\treturn self.flush()")
        fact["relation"] = relation()
        fact["relation"]["occurrences"] = [site(text="\t\treturn self.flush()")]
        decoded = json.loads(json.dumps(registered))
        self.assertEqual(validate_registration(decoded), registered)
        self.assertEqual(decoded["tasks"][0]["facts"][0]["source"]["text"], "\t\treturn self.flush()")
        with tempfile.TemporaryDirectory() as directory:
            Path(directory, "store.py").write_bytes(source)
            verified = _verify_sources(decoded, {"sample": directory})
            self.assertEqual(verified["sample"]["store.py"], source)
            decoded["tasks"][0]["facts"][0]["source"]["text"] = "\t\treturn self.wrong()"
            with self.assertRaisesRegex(ValueError, "pinned source line"):
                _verify_sources(decoded, {"sample": directory})

    def test_source_witness_tabs_do_not_admit_other_controls_or_tabbed_labels(self):
        for control in ("\n", "\r", "\0", "\x1b", "\b"):
            for location in ("source", "occurrence"):
                with self.subTest(control=repr(control), location=location):
                    registered = registration()
                    fact = registered["tasks"][0]["facts"][0]
                    fact["relation"] = relation()
                    target = fact["source"] if location == "source" else fact["relation"]["occurrences"][0]
                    target["text"] = "\treturn self.flush()" + control
                    with self.assertRaisesRegex(ValueError, "control character"):
                        validate_registration(registered)
        registered = registration()
        registered["tasks"][0]["relevant"][0]["identity"]["symbol"] = "sa\tve"
        with self.assertRaisesRegex(ValueError, "control character"):
            validate_registration(registered)

    def test_source_frozen_metric_cohorts_cannot_select_wrong_types_or_change_precision_panel(self):
        registered = registration()
        registered["samplingDesign"] = "Preserve original tasks; source-only predeclared metric cohorts."
        registered["metricCohorts"] = {
            metric: dict(taskIds=["save"], rationale="Synthetic schema fixture only.",
                         minimumRepositories=8, minimumIndependentClusters=80 if metric == "broadSuccess" else 1)
            for metric in ("broadSuccess", "exactDirected", "claimPrecisionEmptyZero")
        }
        # The broad sample cannot also be an exact/directed control.
        with self.assertRaisesRegex(ValueError, "incorrectly typed"):
            validate_registration(registered)
        exact = dict(task("exact"), id="exact-save")
        registered["tasks"].append(exact)
        registered["metricCohorts"]["exactDirected"]["taskIds"] = ["exact-save"]
        self.assertEqual(validate_registration(registered), registered)
        registered["metricCohorts"]["claimPrecisionEmptyZero"]["taskIds"] = ["exact-save"]
        with self.assertRaises(ValueError):
            validate_registration(registered)

    def test_duplicate_json_keys_nonfinite_and_unknown_major_are_rejected(self):
        for raw in (b'{"a":1,"a":2}', b'{"a":NaN}'):
            with self.assertRaises(ValueError):
                _json(raw)
        wrong = dict(registration(), schema="compass.broad-query-registration/99")
        with self.assertRaises(ValueError):
            validate_registration(wrong)


class BundleTests(unittest.TestCase):
    """Local fake captures exercise provenance without executing either product."""
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        source = self.root / "source"
        source.mkdir()
        source.joinpath("store.py").write_bytes(b"class Store:\n    def save(self):\n        return self.flush()\n    def flush(self):\n        pass\n")
        self.source = source

    def put(self, path, value):
        data = value if isinstance(value, bytes) else json.dumps(value, sort_keys=True).encode()
        self.root.joinpath(path).write_bytes(data)
        return dict(path=path, sha256=sha(data))

    def bundle(self):
        reg = registration()
        reg["arms"] = [dict(id="default"), dict(id="normalized", measure="utf8_bytes", budgetBytes=128, overflow="fail")]
        registered = self.put("registration.json", reg)
        environment = self.put("environment.json", dict(python="fixture; no installed competitor"))
        executable = self.put("executable", b"fixture; never executed")
        normalizer = self.put("normalizer.py", b"# fixture normalization, not a production parser\n")
        release = self.put("release.json", dict(version="0.0-fixture", commit="b" * 40,
                                               checkedAt="2026-01-01T00:00:01Z",
                                               source="https://github.com/Graphify-Labs/graphify/releases"))
        tools = [dict(id=tool, version="0.0-fixture", commit="b" * 40, executable=executable,
                      environment=environment, normalizer=normalizer, nativeOffline=True,
                      settings=dict(scope="registered source inventory")) for tool in ("compass", "graphify")]
        tools[1]["releaseVerification"] = release
        graphs = []
        for tool in ("compass", "graphify"):
            graphs.append(dict(repository="sample", tool=tool, artifact=self.put(tool+"-graph.json", graph(tool)),
                               sourceCommit="a" * 40, sourceCleanBeforeAndAfter=True,
                               sourceInventory=self.put(tool+"-inventory.json", dict(files=reg["repositories"][0]["files"])),
                               buildCapture=self.put(tool+"-build.json", dict(argv=[tool, "extract", str(self.source)],
                                   exit_code=0, timed_out=False, output_limited=False, wall_ms=10,
                                   stdout_bytes=5, stderr_bytes=0,
                                   stdout=self.put(tool+"-build.stdout", b"built"),
                                   stderr=self.put(tool+"-build.stderr", b"")))))
        observations, reviews = [], []
        for tool in ("compass", "graphify"):
            for arm in ARMS:
                prefix = tool + "-" + arm
                out = self.put(prefix+".stdout", b"save the state")
                err = self.put(prefix+".stderr", b"")
                capture = self.put(prefix+"-capture.json", dict(argv=[tool, "query", task()["question"]],
                                  exit_code=0, timed_out=False, output_limited=False, wall_ms=20,
                                  stdout_bytes=14, stderr_bytes=0, stdout=out, stderr=err))
                answer = response()
                span = dict(startByte=0, endByte=4, sha256=sha(b"save"))
                answer["ranked"][0]["span"] = span
                answer["claims"][0]["span"] = span
                answer["schema"] = NORMALIZED_SCHEMA
                answer.update(task="save", tool=tool, arm=arm)
                normalized = self.put(prefix+"-normalized.json", answer)
                observations.append(dict(task="save", tool=tool, arm=arm, normalized=normalized,
                                         captures=[capture], condition="fresh_cli", peakRssBytes=4096))
                reviews.append(dict(judgment(), task="save", tool=tool, arm=arm,
                                    normalizedSha256=normalized["sha256"],
                                    independentReview=dict(status="pending", reviewer=None, toolLabelsBlinded=True)))
        run = self.put("run.json", dict(schema=RUN_SCHEMA, registrationSha256=registered["sha256"],
                       startedAt="2026-01-01T00:00:02Z", tools=tools, graphs=graphs,
                       arms=reg["arms"], observations=observations))
        reviewed = self.put("reviews.json", dict(schema=REVIEW_SCHEMA, runSha256=run["sha256"], observations=reviews))
        return dict(schema=MANIFEST_SCHEMA, registration=registered, run=run, reviews=reviewed,
                    sourceRoots=dict(sample=str(self.source)), graphQualification=[])

    def test_complete_diagnostic_remains_ineligible_without_independent_heldout_evidence(self):
        result = assess(self.bundle(), self.root)
        self.assertFalse(result["eligibleForSuperiorityClaim"])
        self.assertEqual(len(result["rows"]), 4)
        self.assertEqual(result["summaries"]["compass"]["normalized"]["answerableSuccess"], 1)
        self.assertTrue(any("power analysis" in f for f in result["failures"]))
        self.assertTrue(any("production graph" in f for f in result["failures"]))
        self.assertTrue(any("reviewer identity/type" in f for f in result["failures"]))

    def test_fabricated_report_metrics_are_recomputed_and_rejected(self):
        manifest = self.bundle()
        path = self.root / "input.json"
        path.write_text(json.dumps(manifest))
        report = self.root / "report.json"
        def quiet_main(args):
            with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
                return main(args)
        self.assertEqual(quiet_main(["--score", str(path), "--output", str(report)]), 0)
        self.assertEqual(quiet_main(["--check", str(report)]), 1)
        data = json.loads(report.read_text())
        data["assessment"]["eligibleForSuperiorityClaim"] = True
        report.write_text(json.dumps(data))
        self.assertEqual(quiet_main(["--check", str(report)]), 2)

    def test_build_raw_counts_and_registered_inventory_cannot_be_fabricated(self):
        for mode in ("missing", "bytes", "inventory"):
            with self.subTest(mode=mode):
                manifest = self.bundle()
                run = _json(self.root.joinpath("run.json").read_bytes())
                entry = run["graphs"][0]
                if mode == "inventory":
                    entry["sourceInventory"] = self.put("wrong-inventory.json", dict(files=[]))
                else:
                    build = _json(self.root.joinpath(entry["buildCapture"]["path"]).read_bytes())
                    if mode == "missing":
                        build.pop("stdout")
                    else:
                        build["stdout_bytes"] = 999
                    entry["buildCapture"] = self.put("changed-build.json", build)
                manifest["run"] = self.put("run.json", run)
                reviews = _json(self.root.joinpath("reviews.json").read_bytes())
                reviews["runSha256"] = manifest["run"]["sha256"]
                manifest["reviews"] = self.put("reviews.json", reviews)
                with self.assertRaises((ValueError, TypeError)):
                    assess(manifest, self.root)

    def test_artifact_hashes_bounds_and_path_containment_are_mandatory(self):
        reference = self.put("capture", b"abc")
        reader = Artifacts(self.root)
        self.assertEqual(reader.read(reference, 3), b"abc")
        for wrong in [dict(reference, sha256="0" * 64), dict(reference, path="../capture")]:
            with self.assertRaises(ValueError):
                reader.read(wrong, 3)
        with self.assertRaises(ValueError):
            reader.read(reference, 2)
        self.root.joinpath("capture").write_bytes(b"abd")
        with self.assertRaises(ValueError):
            reader.recheck()

    def test_over_budget_capture_is_a_retained_failed_row_not_a_cheaper_success(self):
        manifest = self.bundle()
        run = _json(self.root.joinpath("run.json").read_bytes())
        capture = _json(self.root.joinpath("compass-normalized-capture.json").read_bytes())
        capture["stdout"] = self.put("compass-normalized.stdout", b"save the state" + b"x" * 130)
        capture["stdout_bytes"] = 144
        changed = self.put("compass-normalized-capture.json", capture)
        for obs in run["observations"]:
            if (obs["tool"], obs["arm"]) == ("compass", "normalized"):
                obs["captures"] = [changed]
        manifest["run"] = self.put("run.json", run)
        reviews = _json(self.root.joinpath("reviews.json").read_bytes())
        reviews["runSha256"] = manifest["run"]["sha256"]
        manifest["reviews"] = self.put("reviews.json", reviews)
        result = assess(manifest, self.root)
        self.assertEqual(result["summaries"]["compass"]["normalized"]["firstPageSuccess"], 0)
        cost = result["costSummaries"]["compass"]["normalized"]
        self.assertEqual(cost["overBudget"], 1)
        self.assertEqual(cost["workflowBytes"], 144)
        self.assertEqual(cost["failures"], 1)

    def test_response_from_a_different_task_cannot_be_reused(self):
        manifest = self.bundle()
        run = _json(self.root.joinpath("run.json").read_bytes())
        normalized = _json(self.root.joinpath("compass-normalized-normalized.json").read_bytes())
        normalized["task"] = "different-task"
        changed = self.put("compass-normalized-normalized.json", normalized)
        for obs in run["observations"]:
            if (obs["tool"], obs["arm"]) == ("compass", "normalized"):
                obs["normalized"] = changed
        manifest["run"] = self.put("run.json", run)
        reviews = _json(self.root.joinpath("reviews.json").read_bytes())
        reviews["runSha256"] = manifest["run"]["sha256"]
        for review in reviews["observations"]:
            if (review["tool"], review["arm"]) == ("compass", "normalized"):
                review["normalizedSha256"] = changed["sha256"]
        manifest["reviews"] = self.put("reviews.json", reviews)
        with self.assertRaisesRegex(ValueError, "different task/tool/arm"):
            assess(manifest, self.root)

    def test_oracle_informed_broad_question_rewrite_is_rejected(self):
        manifest = self.bundle()
        run = _json(self.root.joinpath("run.json").read_bytes())
        capture = _json(self.root.joinpath("compass-normalized-capture.json").read_bytes())
        capture["argv"] = ["compass", "query", "Store.save exact expected implementation"]
        changed = self.put("compass-normalized-capture.json", capture)
        for obs in run["observations"]:
            if (obs["tool"], obs["arm"]) == ("compass", "normalized"):
                obs["captures"] = [changed]
        manifest["run"] = self.put("run.json", run)
        reviews = _json(self.root.joinpath("reviews.json").read_bytes())
        reviews["runSha256"] = manifest["run"]["sha256"]
        manifest["reviews"] = self.put("reviews.json", reviews)
        with self.assertRaisesRegex(ValueError, "rewritten"):
            assess(manifest, self.root)

    def test_source_drift_missing_matrix_and_late_registration_are_errors(self):
        for variant in ("source", "matrix", "late"):
            with self.subTest(variant=variant):
                manifest = self.bundle()
                if variant == "source":
                    self.source.joinpath("store.py").write_text("changed\n")
                else:
                    run = _json(self.root.joinpath("run.json").read_bytes())
                    if variant == "matrix":
                        run["observations"].pop()
                    else:
                        run["startedAt"] = "2025-01-01T00:00:00Z"
                    manifest["run"] = self.put("run.json", run)
                    reviews = _json(self.root.joinpath("reviews.json").read_bytes())
                    reviews["runSha256"] = manifest["run"]["sha256"]
                    manifest["reviews"] = self.put("reviews.json", reviews)
                with self.assertRaises(ValueError):
                    assess(manifest, self.root)
                self.source.joinpath("store.py").write_bytes(b"class Store:\n    def save(self):\n        return self.flush()\n    def flush(self):\n        pass\n")


if __name__ == "__main__":
    unittest.main()
