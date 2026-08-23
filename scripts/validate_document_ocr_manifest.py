#!/usr/bin/env python3
"""Validate the bounded, redistributable document-OCR qualification manifest."""

from __future__ import annotations

import json
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "tests/qualification/document-ocr/v1/manifest.json"


def fail(message: str) -> None:
    raise SystemExit(f"document OCR manifest error: {message}")


def main() -> None:
    value = json.loads(MANIFEST.read_text(encoding="utf-8"))
    if value.get("schema") != "compass.document-ocr.qualification/1":
        fail("unsupported schema")
    if value.get("license") != "CC0-1.0 synthetic fixtures":
        fail("fixture license is not the reviewed CC0 identity")
    if value.get("offline_default") is not True or value.get("network_during_tests") is not False:
        fail("offline/network policy changed")
    required_formats = {"pdf", "docx", "pptx", "xlsx", "png"}
    if set(value.get("formats", [])) != required_formats:
        fail("format set changed")
    if value.get("engine") != "oar-ocr/0.9.2":
        fail("engine identity changed")
    if value.get("model_revision") != "GreatV/oar-ocr@v0.7.0":
        fail("model revision is not immutable v0.7.0")
    if value.get("renderer") != "hayro/0.7.1@300dpi":
        fail("renderer identity changed")
    preprocessing = value.get("preprocessing")
    if preprocessing != {
        "version": 2,
        "exif_orientation": True,
        "alpha_background": "white",
        "resize_filter": "triangle",
        "engine_max_side": 2048,
        "tile_overlap": 128,
        "engine_threads": 1,
        "document_timeout_seconds": 600,
    }:
        fail("preprocessing identity changed")
    gate = value.get("installed_model_gate")
    if not isinstance(gate, dict) or gate.get("maximum_cer_bps") != 500:
        fail("clean-English CER gate changed")
    blockers = value.get("blocking_unmeasured_release_gates")
    if not isinstance(blockers, list) or len(blockers) != 5 or not all(
        isinstance(item, str) and item for item in blockers
    ):
        fail("unmeasured release blockers are not explicit")
    print(json.dumps({
        "schema": value["schema"],
        "manifest": str(MANIFEST.relative_to(ROOT)),
        "offline": True,
        "engine": value["engine"],
        "model_revision": value["model_revision"],
        "renderer": value["renderer"],
        "clean_english_maximum_cer_bps": gate["maximum_cer_bps"],
        "preprocessing_version": preprocessing["version"],
        "blocking_unmeasured_release_gates": blockers,
    }, sort_keys=True, separators=(",", ":")))


if __name__ == "__main__":
    main()
