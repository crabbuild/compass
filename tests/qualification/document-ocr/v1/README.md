# Document OCR v1 qualification corpus

This directory is the reviewable manifest for the license-safe document/OCR
gate. Deterministic unit and CLI fixtures generate their minimal PDF/OOXML/image
bytes in memory so ZIP metadata and raster bytes remain reproducible and no
opaque office document is committed.

The fixture-only gate covers native DOCX order, sparse typed XLSX cells,
relationship-ordered PPTX parsing, bounded PDF raster dimensions, archive/XML
rejection, selective `auto` policy, provenance-preserving fake OCR, partial
coverage, EXIF orientation, deterministic tiling/overlap reassembly,
cancellation, artifact cache replay/corruption, Unicode-safe semantic slicing,
CLI schemas, missing-model diagnostics, and offline model
listing/verification.

The optional installed-model gate validates the exact pinned OAR/PP-OCR runtime
without downloading. It skips only when `pp-ocrv6-small` is not already
verified in the Compass model cache, and enforces the 5% CER ceiling for the
checked clean-English synthetic raster.

Run:

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-document-ocr \
  scripts/qualify_document_ocr_v1.sh --fixtures-only
```

Fixture provenance and expected policies are machine-readable in
`manifest.json`. The gate makes no broader multilingual, degraded-document,
handwriting, or cross-platform quality claim from its synthetic smoke fixture.
The same manifest lists every unmeasured release blocker so a narrow passing
smoke test cannot be mistaken for full Plan-022 qualification.
