import copy
from pathlib import Path
import tempfile
import unittest

from benchmarks.agent_query.community_tasks import audit_graph, read_bounded


class CommunityTaskTests(unittest.TestCase):
    limits = dict(maxNodes=100, maxEdges=100)

    def tasks(self):
        return [dict(id="task-a", declarations=[
            dict(id="a", file="a.rs", startLine=10, symbol="alpha"),
            dict(id="b", file="b.rs", startLine=20, symbol="beta"),
        ]), dict(id="task-b", declarations=[
            dict(id="c", file="a.rs", startLine=30, symbol="gamma"),
        ])]

    def graph(self, tool="compass"):
        nodes = []
        for identifier, name, file, line, group in [
            ("a", "alpha", "a.rs", 10, 0),
            ("b", "beta", "b.rs", 20, 0),
            ("c", "gamma", "a.rs", 30, 1),
        ]:
            if tool == "compass":
                nodes.append(dict(id=identifier, name=name, source=dict(file=file, startLine=line), community=dict(id=group)))
            else:
                nodes.append(dict(id=identifier, label=name + "()", source_file=file,
                                  source_location=f"L{line}", community=group))
        return dict(nodes=nodes, links=[])

    def test_zero_is_a_real_community_and_pair_denominators_are_separate(self):
        for tool in ["compass", "graphify"]:
            result = audit_graph(self.graph(tool), tool, self.tasks(), self.limits)
            self.assertEqual(result["summaries"]["within_task"]["same_community"], 1)
            self.assertEqual(result["summaries"]["cross_task"]["different_community"], 2)
            self.assertEqual(result["summaries"]["within_task/cross_file"]["same_community"], 1)
            self.assertEqual(result["declarations"][0]["communityNodes"], 2)
            self.assertEqual(result["declarations"][0]["communitySourceFiles"], 2)

    def test_node_order_and_community_renumbering_do_not_change_pair_outcomes(self):
        graph = self.graph()
        first = audit_graph(graph, "compass", self.tasks(), self.limits)
        graph["nodes"].reverse()
        for node in graph["nodes"]:
            node["community"]["id"] += 90
        second = audit_graph(graph, "compass", self.tasks(), self.limits)
        self.assertEqual(first["pairs"], second["pairs"])
        self.assertEqual(first["summaries"], second["summaries"])

    def test_missing_and_unassigned_nodes_are_not_separated_successes(self):
        for mode in ["missing", "unassigned"]:
            graph = self.graph()
            if mode == "missing":
                graph["nodes"].pop(1)
            else:
                graph["nodes"][1].pop("community")
            result = audit_graph(graph, "compass", self.tasks(), self.limits)
            self.assertEqual(result["declarations"][1]["status"], mode)
            self.assertEqual(result["summaries"]["within_task"]["unresolved"], 1)
            self.assertEqual(result["summaries"]["cross_task"]["unresolved"], 1)

    def test_duplicate_anchor_preserves_both_identities_as_ambiguous(self):
        graph = self.graph()
        duplicate = copy.deepcopy(graph["nodes"][0])
        duplicate["id"] = "a-second"
        duplicate["community"]["id"] = 1
        graph["nodes"].append(duplicate)
        result = audit_graph(graph, "compass", self.tasks(), self.limits)
        self.assertEqual(result["declarations"][0]["status"], "ambiguous")
        self.assertEqual(result["declarations"][0]["matchedNodeIds"], ["a", "a-second"])
        self.assertEqual(result["summaries"]["within_task"]["unresolved"], 1)

    def test_exact_start_and_name_are_required_not_container_extent_or_substring(self):
        for change in [dict(name="alphabet"), dict(source=dict(file="a.rs", startLine=1, endLine=50))]:
            graph = self.graph()
            graph["nodes"][0].update(change)
            result = audit_graph(graph, "compass", self.tasks(), self.limits)
            self.assertEqual(result["declarations"][0]["status"], "missing")

    def test_boolean_and_negative_communities_fail_closed(self):
        for tool in ["compass", "graphify"]:
            for value in [False, -1, "0"]:
                graph = self.graph(tool)
                graph["nodes"][0]["community"] = dict(id=value) if tool == "compass" else value
                with self.assertRaises(ValueError):
                    audit_graph(graph, tool, self.tasks(), self.limits)

    def test_duplicate_node_ids_and_repeated_source_judgments_fail(self):
        graph = self.graph()
        graph["nodes"].append(copy.deepcopy(graph["nodes"][0]))
        with self.assertRaises(ValueError):
            audit_graph(graph, "compass", self.tasks(), self.limits)
        tasks = self.tasks()
        tasks[1]["declarations"] = copy.deepcopy(tasks[0]["declarations"])
        with self.assertRaises(ValueError):
            audit_graph(self.graph(), "compass", tasks, self.limits)

    def test_duplicate_task_ids_and_large_pair_sets_fail(self):
        tasks = self.tasks()
        tasks[1]["id"] = tasks[0]["id"]
        with self.assertRaises(ValueError):
            audit_graph(self.graph(), "compass", tasks, self.limits)
        with self.assertRaises(ValueError):
            audit_graph(self.graph(), "compass", [dict(id="a", declarations=[{}] * 257)], self.limits)

    def test_graph_and_file_bounds_are_errors_not_empty_evidence(self):
        with self.assertRaises(ValueError):
            audit_graph(self.graph(), "compass", self.tasks(), dict(maxNodes=2, maxEdges=100))
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "input.json"
            path.write_bytes(b"abcde")
            self.assertEqual(read_bounded(path, 5), b"abcde")
            with self.assertRaises(ValueError):
                read_bounded(path, 4)
            for limit in [-1, False, 10**12]:
                with self.assertRaises(ValueError):
                    read_bounded(path, limit)

    def test_boolean_source_line_is_not_an_integer_anchor(self):
        tasks = self.tasks()
        tasks[0]["declarations"][0]["startLine"] = True
        with self.assertRaises(ValueError):
            audit_graph(self.graph(), "compass", tasks, self.limits)


if __name__ == "__main__":
    unittest.main()
