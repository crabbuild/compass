---
meta:
  contentType: Guide
  title: Qualifying document OCR
  navLabel: Document OCR Qualification
  category: Implementation
  overview: Reproduce the offline document and optional installed-model OCR gates.
---

# Qualifying document OCR

The default gate is offline and model-free:

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-document-ocr \
  scripts/qualify_document_ocr_v1.sh --fixtures-only
```

It checks the versioned OCR and document contracts, archive and raster limits,
native DOCX/PPTX/XLSX order, selective OCR policy, response rejection,
provenance, EXIF orientation, alpha compositing, deterministic resize/tiling
and overlap reassembly, cancellation, partial completeness, prepared-document
caching, Unicode-safe semantic slicing, CLI JSON, and offline model
verification. It must pass even when the Compass model cache is empty.

Install and verify a profile only for the opt-in native-runtime gate:

```bash
compass models install pp-ocrv6-small
compass models verify pp-ocrv6-small
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-document-ocr \
  scripts/qualify_document_ocr_v1.sh --with-installed-model
```

Installation is the only network step. The gate itself performs no download.
It verifies the immutable OAR-OCR 0.9.2 / `v0.7.0` profile, executes inference,
validates geometry and profile identity, and enforces at most 5% CER on the
deterministic clean-English smoke raster. The smoke raster is CC0 synthetic.

The v1 manifest intentionally does not claim photographed pages, handwriting,
table reconstruction, or any script/platform absent from measured fixtures.
A release claim for those classes requires checked ground truth, CER/WER,
region IoU, reading-order and native-duplicate metrics, hostile-input results,
and equivalent recognized content on every supported CPU architecture. Vendor
benchmark figures are not Compass qualification evidence.

The manifest machine-reports the remaining blocking, unmeasured release gates:
the pinned Tesseract degraded-English comparison, PP-OCRv6 medium and `ocrs`
candidate comparison, per-script quality/geometry, x86_64/aarch64 equivalence,
and hostile-corpus RSS/timeout evidence. The implementation is usable with the
narrow clean-English gate, but Plan 022 must not be marked `DONE` until those
external qualification runs and its prerequisite plan statuses are reviewed.
