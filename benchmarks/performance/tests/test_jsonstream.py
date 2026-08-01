from __future__ import annotations

import json
from pathlib import Path
import tempfile
import unittest

from benchmarks.performance.compass.jsonstream import (
    iter_top_level_array,
    iter_top_level_object_array,
    read_top_level_object_value,
    read_top_level_value,
)


class JsonStreamTests(unittest.TestCase):
    def write(self, directory: str, value: str) -> Path:
        path = Path(directory) / "graph.json"
        path.write_text(value, encoding="utf-8")
        return path

    def test_one_character_chunks_preserve_strings_and_unicode(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            raw = json.dumps(
                {
                    "graph": {"note": "brace } and quote \""},
                    "nodes": [{"id": "前端", "label": "a,{b}"}],
                    "links": [],
                },
                ensure_ascii=False,
            )
            path = self.write(directory, raw)
            self.assertEqual(
                list(iter_top_level_array(path, "nodes", chunk_chars=1)),
                [{"id": "前端", "label": "a,{b}"}],
            )
            self.assertEqual(read_top_level_value(path, "graph")["note"], 'brace } and quote "')

    def test_nested_value_does_not_decode_large_parent_member(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = self.write(
                directory,
                '{"graph":{"communities":{"payload":"'
                + ("x" * (17 * 1024 * 1024))
                + '"},"diagnostics":[{"severity":"error"}]},"nodes":[]}',
            )

            diagnostics = read_top_level_object_value(path, "graph", "diagnostics")

        self.assertEqual(diagnostics, [{"severity": "error"}])

    def test_nested_array_streams_members_without_decoding_the_collection(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = self.write(
                directory,
                json.dumps(
                    {
                        "graph": {
                            "note": "before",
                            "diagnostics": [
                                {"severity": "info", "code": "first"},
                                {"severity": "error", "code": "second"},
                            ],
                            "after": {"nested": True},
                        },
                        "nodes": [],
                    }
                ),
            )

            diagnostics = list(
                iter_top_level_object_array(
                    path, "graph", "diagnostics", chunk_chars=1
                )
            )

        self.assertEqual(
            diagnostics,
            [
                {"severity": "info", "code": "first"},
                {"severity": "error", "code": "second"},
            ],
        )

    def test_edges_fallback_is_a_caller_decision(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = self.write(directory, '{"nodes":[],"edges":[{"source":"a","target":"b"}]}')
            with self.assertRaises(KeyError):
                list(iter_top_level_array(path, "links", chunk_chars=2))
            self.assertEqual(len(list(iter_top_level_array(path, "edges", chunk_chars=2))), 1)

    def test_non_object_array_item_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = self.write(directory, '{"nodes":[1]}')
            with self.assertRaisesRegex(ValueError, "non-object"):
                list(iter_top_level_array(path, "nodes", chunk_chars=1))

    def test_truncated_json_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = self.write(directory, '{"nodes":[{"id":"a"}')
            with self.assertRaisesRegex(ValueError, "invalid|truncated"):
                list(iter_top_level_array(path, "nodes", chunk_chars=2))

    def test_duplicate_requested_key_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = self.write(directory, '{"nodes":[],"nodes":[]}')
            with self.assertRaisesRegex(ValueError, "duplicate"):
                list(iter_top_level_array(path, "nodes", chunk_chars=2))

if __name__ == "__main__":
    unittest.main()
