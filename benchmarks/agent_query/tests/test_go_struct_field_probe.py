import unittest

from benchmarks.agent_query.go_struct_field_probe import compare


FIELD = {"file": "store.go", "struct": "Store", "structLine": 3,
         "field": "Value", "fieldLine": 4}


def graph(*, shared=False, duplicate=False, field_kind="field", field_line=4):
    nodes = [
        {"id": "owner", "kind": "struct", "name": "Store",
         "source": {"file": "store.go", "startLine": 3}},
        {"id": "field", "kind": field_kind, "name": "Value",
         "source": {"file": "store.go", "startLine": field_line}},
    ]
    links = [{"source": "owner", "target": "field", "kind": "contains"}]
    if shared:
        nodes.append({"id": "other", "kind": "struct", "name": "Other",
                      "source": {"file": "store.go", "startLine": 10}})
        links.append({"source": "other", "target": "field", "kind": "contains"})
    if duplicate:
        nodes.append({"id": "duplicate", "kind": "field", "name": "Value",
                      "source": {"file": "store.go", "startLine": 4}})
        links.append({"source": "owner", "target": "duplicate", "kind": "contains"})
    return {"directed": True, "nodes": nodes, "links": links}


class GoStructFieldProbeTests(unittest.TestCase):
    def test_exact_source_field_and_unique_owner_required(self):
        self.assertEqual(compare(graph(), "compass", [FIELD])["statusCounts"], {"matched": 1})
        self.assertEqual(compare(graph(shared=True), "compass", [FIELD])["statusCounts"],
                         {"ambiguous-field": 1})
        self.assertEqual(compare(graph(duplicate=True), "compass", [FIELD])["statusCounts"],
                         {"ambiguous-field": 1})
        self.assertEqual(compare(graph(field_kind="method"), "compass", [FIELD])["statusCounts"],
                         {"missing-field": 1})
        self.assertEqual(compare(graph(field_line=5), "compass", [FIELD])["statusCounts"],
                         {"missing-field": 1})

    def test_graphify_field_type_context_is_not_a_declaration(self):
        graphify = {"directed": False, "nodes": [
            {"id": "owner", "label": "Store", "source_file": "store.go", "source_location": "L3"},
            {"id": "type", "label": "int", "source_file": "store.go", "source_location": "L4"}],
            "links": [{"source": "owner", "target": "type", "relation": "references",
                       "context": "field", "source_file": "store.go", "source_location": "L4"}]}
        result = compare(graphify, "graphify", [FIELD])
        self.assertEqual(result["statusCounts"], {"missing-field": 1})
        self.assertEqual(result["fieldTypeContextRecords"], 1)


if __name__ == "__main__":
    unittest.main()
