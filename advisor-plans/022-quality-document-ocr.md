# Plan 022: Add bounded, quality-gated OCR to document processing

> **Executor instructions**: Read this plan completely before editing. It is a
> program plan that extends Plans 006–010; do not reimplement their document
> artifact, chunking, or OOXML work. Execute the phases in order, run every
> verification, and stop on a listed condition instead of substituting an
> unreviewed model, renderer, runtime, or cloud OCR service. When complete,
> update this plan's row in `advisor-plans/README.md` unless a reviewer owns the
> index.
>
> **Drift check (run first)**:
> `git diff --stat 3471678d..HEAD -- Cargo.toml Cargo.lock crates/compass-ocr crates/compass-media crates/compass-files crates/compass-semantic crates/compass-core crates/compass-history crates/compass-cli tests/qualification/document-ocr scripts docs CHANGELOG.md COMPATIBILITY.md MIGRATION.md PERFORMANCE.md`
> Plans 006, 007, 008, and 010 must be complete. Compare their target contracts
> with live code. If the live document artifact, locator, completeness, or cache
> contract differs materially from the assumptions below, stop and revise this
> plan with a reviewer.

## Status

- **Priority**: P1
- **Effort**: XL
- **Risk**: HIGH
- **Depends on**: `advisor-plans/006-harden-media-ingestion.md`,
  `advisor-plans/007-versioned-document-artifact.md`,
  `advisor-plans/008-lossless-document-fusion.md`, and
  `advisor-plans/010-native-ooxml-documents.md`
- **Category**: direction, security, perf, tests
- **Planned at**: commit `3471678d`, 2026-08-23
- **Execution**: IN PROGRESS — production processing and the clean-English
  installed-model gate are implemented. Full release qualification remains
  blocked by the machine-reported candidate, multilingual/degraded,
  cross-architecture, and hostile-corpus measurements, plus review of the
  prerequisite plan statuses.

## Why this matters

Native PDF extraction reads an existing text layer, and native OOXML extraction
reads XML. Neither recovers text from scanned PDF pages, screenshots, photographed
documents, or images embedded in DOCX, PPTX, and XLSX. Treating those inputs as
empty loses important evidence; OCRing every page indiscriminately duplicates
better native text, increases cost, and can replace exact evidence with model
errors.

This plan adds selective, local OCR as a derived evidence channel. It keeps the
credential-free structural path intact, makes model and process boundaries
explicit, preserves page/slide/sheet/image geometry and confidence, and qualifies
quality against a pinned corpus before publishing a support claim. OCR must never
execute document content, contact a service during extraction, or silently
overwrite native text.

## Relationship to the existing document program

This plan does not replace Plans 006–012:

- Plan 006 makes corrupt and over-limit media fail explicitly.
- Plan 007 introduces `compass.document/1`, typed blocks, logical locators,
  diagnostics, completeness, and normalizer-version cache identity.
- Plan 008 decodes rich documents before packing, slices normalized content
  losslessly, and fuses deterministic structure with optional semantic evidence.
- Plan 010 preserves DOCX/XLSX/PPTX structure and exposes embedded media as
  unsupported evidence rather than executing it.
- Plan 012 qualifies the credential-free document graph. Its base gate must
  remain independent of OCR models and external helpers.

Plan 022 consumes those boundaries. OCR qualification is an additional gate;
failure or absence of OCR must never invalidate native document support.

## Current state

- `crates/compass-media/src/lib.rs:32-67` dispatches PDF, DOCX, and XLSX to
  bounded text conversion. PDF uses `oxidize-pdf` text extraction only.
- `crates/compass-media/src/lib.rs:74-153` flattens DOCX/XLSX into Markdown-like
  text. Plans 007 and 010 are expected to replace this with ordered artifacts.
- `crates/compass-files/src/detect.rs:24-30` discovers DOCX/XLSX but not PPTX;
  Plan 010 owns PPTX discovery.
- `crates/compass-semantic/src/lib.rs:790-838` reads media into semantic input;
  Plan 008 moves rich-document decoding ahead of semantic packing.
- `Cargo.toml:75` pins `oxidize-pdf` with no OCR dependency. The repository has
  no Tesseract, PaddleOCR, `ocrs`, ONNX Runtime, PDFium, or other OCR engine.
- `crates/compass-transcribe/src/models.rs` is the closest model-acquisition
  exemplar: it pins artifact revision, size, digest, cache location, bounded
  HTTPS download, temporary writes, verification markers, and typed failures.
  Match that behavior; do not move transcription ownership into OCR or create a
  generic model framework before two consumers prove an exact shared contract.
- `docs/design/principles.md` requires local-first structural behavior,
  evidence/provenance, deterministic identities, bounded work, coherent
  publication, explicit machine contracts, and compatibility review.
- `docs/design/security-and-privacy.md` treats repository files, archives,
  images, models, subprocess output, and provider content as untrusted. A limit
  error cannot become an empty successful document.

## Design decisions

### 1. Native text remains authoritative

DOCX, PPTX, and XLSX XML text is extracted directly. A born-digital PDF's text
layer is extracted directly. OCR is considered only for:

1. a PDF page selected by the explicit OCR policy;
2. an image part referenced by DOCX, PPTX, or XLSX;
3. a directly discovered raster image when a caller explicitly requests OCR.

Compass must not render a whole Office document and OCR it. Doing so would
discard exact package structure, duplicate text, and make visual rendering a
new correctness dependency.

### 2. OCR is derived evidence, not exact extraction

Every accepted OCR block carries:

- `origin = "ocr"` or the live typed equivalent;
- source document and page/part/shape/sheet locator;
- pixel polygon or rectangle in a documented coordinate space;
- engine, engine version, model/profile ID, model artifact digests, language
  hints, and preprocessing-policy version;
- bounded confidence as an integer in `0..=10_000`, not a serialized float;
- stable diagnostic codes for low confidence, conflict, truncation, or failure.

OCR text never replaces a native block. Matching OCR may corroborate native
text; conflicting OCR remains separate evidence with a diagnostic. Graph IDs
must be based on source identity, logical source locator, quantized geometry,
and occurrence—not recognized text or confidence.

### 3. OCR is off by default and local when enabled

Public policy values are:

```text
off      native document processing only; no model loading
auto     OCR only eligible scanned/low-text PDF pages and embedded images
always   OCR every eligible PDF page and embedded image within limits
```

`off` is the compatibility default. `auto` and `always` require an installed,
verified local engine profile. Extraction never auto-downloads a model and
never falls back to a remote provider. Model installation is a separate,
visible network command.

If OCR is explicitly requested but its engine/model is unavailable, fail before
publishing rather than silently behaving like `off`. Under existing
`--allow-partial` policy, per-page/per-image failures may publish only when the
artifact records partial visual coverage and the failed locators exactly.

### 4. Quality is selected by Compass evidence, not vendor claims

The recommended high-quality candidate is a pinned PP-OCRv6 small/medium
detector-recognizer profile because the current official PaddleOCR project
documents unified multilingual recognition, orientation/unwarping support, and
improved detection/recognition over PP-OCRv5. It is a candidate, not an
automatic dependency.

Phase 0 must compare:

- PP-OCRv6 small and medium using the official local pipeline as the quality
  reference;
- Tesseract 5 with pinned `tessdata_best` as a reproducible classical baseline;
- current `ocrs`/RTen as a pure-Rust portability candidate.

The current `ocrs` project calls itself early preview and currently recognizes
the Latin alphabet only. Do not choose it as Compass's recommended multilingual
backend solely because it is Rust. Prefer a native PP-OCRv6 inference path only
if the exact exported model runs with bounded, cross-platform behavior and
meets the quality gate. If that is not viable, ship PP-OCRv6 as an explicit
local helper-process backend; Python/helper installation remains optional and
must not enter the normal structural path.

Do not use a generative document VLM as the v1 OCR source. Models such as
PaddleOCR-VL may be evaluated later for table/formula/chart interpretation, but
their generated structure must not be published as exact OCR geometry.

### 5. PDF rasterization is a separate qualified boundary

OCR consumes pixels. Add a `PageRasterizer` boundary rather than coupling the
OCR engine to PDF parsing. Prefer the already-pinned pure-Rust `oxidize-pdf`
family only after confirming that the live pinned or reviewed upgraded version
can render the qualification corpus safely and consistently. Do not silently
add PDFium, MuPDF, Poppler, a browser, or an operating-system renderer.

If no dependency-free renderer passes the gate, the executor must stop after
shipping embedded-image OCR and present a separate explicit optional-renderer
decision. Scanned-PDF OCR must not be claimed without qualified page rendering.

## Target architecture

```text
bounded source bytes
  |
  +--> native document decoder --------------------------+
  |      text, hierarchy, links, native locators          |
  |                                                       |
  `--> bounded raster candidates                          |
         PDF page | DOCX/PPTX/XLSX image part             |
                 |                                        |
                 v                                        |
         OCR policy: off | auto | always                  |
                 |                                        |
                 v                                        |
         page/image rasterization and normalization       |
                 |                                        |
                 v                                        |
         versioned local OCR engine                       |
                 |                                        |
                 v                                        |
         validated OcrObservation regions                 |
                 |                                        |
                 v                                        v
          geometry-aware native/OCR fusion --> DocumentArtifact
                                                   |
                                  +----------------+---------------+
                                  v                                v
                         structural document graph        semantic slices
```

### Crate ownership

- New `compass-ocr` owns engine-neutral request/result types, OCR engine
  implementations, model manifests/acquisition, helper-process protocol,
  preprocessing policy, runtime limits, and OCR qualification helpers.
- `compass-media` owns extraction of raster candidates from PDF/OOXML,
  document-specific locators, page rasterization, and fusion of validated OCR
  observations into `DocumentArtifact`.
- `compass-files` owns discovery, source fingerprints, and atomic/cache
  primitives; it does not run models or parse OCR output.
- `compass-core` sequences document decode, optional OCR, structural
  publication, and coherent failure/partial policy.
- `compass-semantic` consumes already-prepared document slices. It must not
  invoke OCR independently or create a second OCR cache.
- `compass-history` fingerprints the selected OCR realization and persists the
  exact completed artifacts. It never downloads models during historical work.
- `compass-cli` owns flags, model-install UX, inspect output, diagnostics, and
  exit behavior. Keep inference and fusion out of command parsing.

No dependency may point upward from `compass-ocr` or `compass-media` into CLI,
core, semantic, history, or output crates.

## Machine contracts

### Engine-neutral OCR contract

Create public types equivalent to:

```rust
pub const OCR_SCHEMA: &str = "compass.ocr/1";
pub const OCR_PROTOCOL_SCHEMA: &str = "compass.ocr.protocol/1";
pub const OCR_POLICY_VERSION: u32 = 1;

pub enum OcrMode {
    Off,
    Auto,
    Always,
}

pub struct OcrProfileIdentity {
    pub engine: String,
    pub engine_version: String,
    pub profile: String,
    pub model_digests: BTreeMap<String, String>,
    pub languages: Vec<String>,
    pub preprocessing_version: u32,
}

pub struct OcrRequest {
    pub schema: String,
    pub request_id: String,
    pub source_kind: OcrSourceKind,
    pub width: u32,
    pub height: u32,
    pub language_hints: Vec<String>,
    pub image_digest: String,
}

pub struct OcrObservation {
    pub ordinal: u32,
    pub polygon: Vec<OcrPoint>,
    pub text: String,
    pub confidence_bps: u16,
    pub script: Option<String>,
    pub orientation_degrees: i16,
}
```

Requirements:

- `OcrPoint` uses checked integer pixels in the normalized raster coordinate
  space. Polygons have 4–16 points, lie inside image bounds, and have nonzero
  area.
- Requests/results contain opaque request IDs, never absolute source paths.
- Result ordering is canonical. Preserve engine order separately when useful,
  then publish deterministic geometric order with an explicit writing-direction
  limitation diagnostic where needed.
- Validation rejects unknown schema majors, duplicate ordinals, invalid UTF-8,
  non-finite data, out-of-bounds polygons, impossible orientation, unknown
  request IDs, excessive regions/text, and model/profile mismatch.
- Messages and diagnostics are bounded and must not echo image bytes or
  uncontrolled helper stderr.

### Document artifact additions

Extend the Plan-007 artifact additively where possible:

- an OCR block/evidence origin;
- an OCR locator containing the owning native locator, raster-candidate ID,
  normalized pixel geometry, and region occurrence;
- OCR profile identity and visual-coverage status;
- `visual_coverage = not_requested | complete | partial | failed`;
- diagnostics including stable codes listed below.

Keep `complete` profile-relative:

- OCR `off`: native extraction may be complete even though visual coverage is
  `not_requested`.
- OCR requested and all selected candidates processed: visual coverage is
  `complete`.
- selected candidates omitted by a soft unsupported case or allowed failure:
  artifact is incomplete and visual coverage is `partial` or `failed`.
- corrupt, over-limit, missing-model, or protocol-invalid cases are typed
  errors unless existing explicit partial policy permits publication.

Required diagnostic codes include:

```text
ocr_candidate_skipped_too_small
ocr_candidate_limit_reached
ocr_native_text_preferred
ocr_native_text_conflict
ocr_low_confidence
ocr_language_unsupported
ocr_pdf_renderer_unavailable
ocr_engine_unavailable
ocr_engine_timeout
ocr_engine_output_rejected
ocr_partial_visual_coverage
ocr_reading_order_approximate
```

### Local helper protocol

If the selected quality backend is a helper process, do not parse a vendor's
human CLI output. Define one Compass-owned protocol:

1. Compass creates a private per-invocation temporary directory.
2. It writes bounded normalized PNG inputs with opaque names and a
   `compass.ocr.protocol/1` request manifest.
3. It launches the configured helper using argument arrays, a restricted
   environment, no shell, a document-level timeout, and capped stdout/stderr.
4. The helper writes one bounded response file in the same directory.
5. Compass validates schema, profile/model identity, request coverage, geometry,
   counts, and digests before accepting any observation.
6. The directory is removed on success and failure. Files use source-equivalent
   permissions and are never written below the repository or `compass-out/`.

The helper receives no original path, credentials, network configuration, or
output directory. Extraction must work with network disabled after model
installation.

## Selection and fusion policy

### PDF `auto` eligibility

Select a page for OCR when at least one source-backed signal is true:

- normalized native visible text has fewer than 24 non-whitespace characters;
- more than 20% of extracted text is replacement/control characters;
- the parser classifies the page as image-dominant and native text has fewer
  than 100 non-whitespace characters;
- text extraction reports a glyph-mapping diagnostic that prevents coherent
  native text.

Store thresholds in `OCR_POLICY_VERSION`; do not expose unrestricted values in
the first public CLI. `always` selects every page within limits. A PDF page is
rasterized at a documented target of 300 DPI, then reduced only as required by
the pixel cap while preserving aspect ratio.

### OOXML image eligibility

Plans 010's OPC reader must expose inert image relationships and exact owning
locators. OCR only decoded raster formats accepted by the image boundary. In
`auto`, skip icons/logos below 64×64 or 4,096 pixels, deduplicate repeated image
bytes by digest, and run text detection on other candidates. Reuse observations
for the same image digest but publish a separate located block for every source
occurrence.

Do not OCR SVG script/content, OLE objects, macros, linked remote images, audio,
or video. External image relationships remain inert links and are never fetched.

### Preprocessing

Preprocessing must be deterministic and versioned:

- decode with strict image dimension/pixel/animation-frame limits;
- apply declared EXIF orientation once;
- composite alpha onto white;
- preserve an RGB source and derive grayscale only when the selected profile
  requires it;
- resize with one fixed algorithm;
- tile images exceeding the engine side limit with fixed overlap and
  deterministic tile order;
- map tile polygons back to the normalized source raster before deduplication;
- leave deskew, orientation classification, and unwarping to a profile only
  when that profile identity records those stages.

Do not apply several heuristic thresholding variants and select the text result
that looks best. That creates an undocumented model ensemble and unstable
meaning.

### Native/OCR fusion

Native text always wins as the primary block. For a PDF page with positioned
native text:

1. normalize a comparison-only copy with Unicode normalization, whitespace
   folding, and case folding; preserve original strings separately;
2. compare only regions whose geometry overlaps;
3. if normalized strings are equal, retain native text and attach OCR
   corroboration metadata without adding duplicate semantic text;
4. if strings differ materially, retain both, mark OCR as derived, and emit
   `ocr_native_text_conflict`;
5. if native geometry is unavailable, do not guess overlap. In `auto`, OCR only
   low-text pages; in `always`, publish a separate OCR channel and diagnose the
   inability to fuse geometrically.

Never choose a string solely because its OCR confidence is higher. Confidence
is engine-local and cannot outrank exact package/PDF evidence.

## Initial resource ceilings

Put these in named limits and test exact-limit plus one-over-limit cases. A
review may lower them based on measurements; raising them requires security and
performance evidence.

| Resource | Initial ceiling |
|---|---:|
| OCR-selected PDF pages per document | 200 |
| OCR-selected OOXML images per document | 256 |
| Decoded pixels per raster | 24,000,000 |
| Aggregate decoded pixels per document | 300,000,000 |
| Raster long edge | 6,000 pixels |
| OCR observations per raster | 10,000 |
| OCR observations per document | 100,000 |
| OCR text per observation | 16 KiB |
| Aggregate OCR text per document | 5,000,000 characters |
| Helper response bytes | 64 MiB |
| Captured helper stderr | 1 MiB |
| Helper wall time per document | 10 minutes |
| Default OCR concurrency | 1 document; bounded engine threads |

Integer arithmetic uses checked operations before allocation. The rasterizer
must calculate `width × height × channels` before allocating. Animated images,
multi-frame TIFFs, and tiled PDFs count each decoded frame/page against the
aggregate limit; v1 processes only the declared first image frame unless the
format contract explicitly says otherwise.

## Cache, fingerprint, and history contract

The prepared-document/OCR cache key must include unambiguous encodings of:

- source byte digest;
- `DOCUMENT_SCHEMA` and `DOCUMENT_NORMALIZER_VERSION`;
- `OCR_SCHEMA` and `OCR_POLICY_VERSION`;
- OCR mode;
- engine/profile/version;
- ordered model artifact digests;
- language hints;
- rasterizer identity/version;
- preprocessing identity/version;
- every meaning-affecting limit below the compiled hard maximum.

Do not include machine-absolute paths, credentials, temporary paths, thread
count, or cache location. A profile change is a hard cache miss; never probe an
older OCR namespace as fallback.

Historical materialization is offline. It may use OCR only when all pinned
model artifacts and the exact allowed local engine are already available. It
must not install models, contact a helper service, or silently switch engines.
The realization fingerprint and artifact registry must preserve exact OCR
profile identity and visual-coverage status. Published realizations remain
immutable.

Because CPU kernels can produce small confidence differences across hardware,
qualification must run on the supported architecture matrix. Quantize
confidence only at the engine boundary. If recognized text or geometry is not
stable enough for deterministic structural publication, keep OCR in the
optional semantic/derived layer and store the exact realization; do not weaken
Compass's structural determinism claim.

## Public CLI contract

Add thin public surfaces after the domain API is stable:

```text
compass document inspect <FILE>
  [--format text|json]
  [--ocr off|auto|always]
  [--ocr-profile <NAME>]
  [--ocr-language <BCP47>]...

compass extract [PATH]
  [--ocr off|auto|always]
  [--ocr-profile <NAME>]
  [--ocr-language <BCP47>]...

compass models list [--format text|json]
compass models install <PINNED_PROFILE>
compass models verify <PINNED_PROFILE>
```

`document inspect --format json` emits `compass.document.inspect/1`, including
artifact schema/version, native blocks, OCR blocks, locators, profile identity,
visual coverage, limits used, and diagnostics. Text output shows page/slide/
sheet/image citations and labels OCR-derived lines visibly.

`models install` is the only OCR command allowed to download. It uses a fixed
HTTPS host allowlist, exact immutable revision, declared byte size, SHA-256,
license/model-card metadata, bounded redirects, atomic temporary files, and a
verified marker. `models verify` performs no network access. Extraction errors
must tell users the exact install/verify command without embedding a URL or
silently downloading.

Do not expose arbitrary executable paths in the first stable CLI. If a helper
backend is required, discover one documented Compass adapter name/version from
an explicit configuration field or controlled environment variable, validate
it, and fingerprint it. Never concatenate a shell command.

## OCR qualification contract

Create a license-safe, reviewable corpus under
`tests/qualification/document-ocr/v1/` with ground truth and provenance for:

- clean 300-DPI scanned English pages;
- low-resolution, skewed, rotated, noisy, and photographed pages;
- multi-column pages and mixed font sizes;
- digits, punctuation, code snippets, URLs, and identifiers;
- supported Latin, CJK, Cyrillic, Arabic/RTL, and Indic samples when the chosen
  profile claims them;
- tables, formulas, chart labels, and screenshots as text detection cases
  without claiming structural interpretation;
- born-digital PDF pages that must not duplicate native text;
- hybrid PDF pages with native and raster text;
- DOCX/PPTX/XLSX embedded images, repeated image bytes, alt text, and multiple
  source occurrences;
- malformed images, decompression/pixel bombs, huge pages, timeout, bad helper
  protocol, low confidence, and model mismatch.

Ground truth must include expected text plus line/word polygons where geometry
is evaluated. Store source/license/generation metadata adjacent to every
non-synthetic fixture. Do not use customer or private documents.

Measure at least:

- character error rate (CER) and word error rate (WER);
- text-region precision/recall at documented IoU;
- reading-order pair accuracy;
- duplicate-native-text rate;
- native/OCR conflict rate;
- page/image success, partial, and rejection counts;
- wall time, peak bounded allocations or measured RSS, and output size;
- repeated-run and x86_64/aarch64 semantic equivalence.

The recommended backend must satisfy all of these release gates:

1. zero replacement or deletion of native text;
2. zero unbounded/panic/crash cases in the hostile corpus;
3. no network activity during extraction;
4. median CER no worse than 5% on clean supported-script scans and 15% on the
   declared degraded set, reported separately per script;
5. at least 15% relative median-CER improvement over the pinned Tesseract
   baseline on the degraded English set, or a documented review decision that
   another quality metric is more representative;
6. no supported fixture class regresses by more than two absolute CER points
   from the PP-OCRv6 reference execution;
7. duplicate-native-text rate is zero on born-digital fixtures;
8. all model, renderer, process, pixel, region, text, and time limits terminate
   with the declared typed result;
9. exact OCR IDs, source locators, model identity, and diagnostic codes are
   stable across repeated runs; recognized content must be semantically equal
   on supported CPU architectures.

Do not publish “better,” “best,” “SOTA,” accuracy, or speed claims unless the
Compass corpus and commands reproduce them. Vendor benchmark numbers are input
to candidate selection, not Compass evidence.

## Commands you will need

Use a checkout-specific external target on every compiling Cargo invocation:

| Purpose | Command | Expected on success |
|---|---|---|
| Volume | `test -d /Volumes/Workspace && test -w /Volumes/Workspace && mkdir -p /Volumes/Workspace/crabbuild-target/compass-bb425f03-document-ocr` | exit 0 |
| OCR crate | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-bb425f03-document-ocr cargo test -p compass-ocr --locked` | exit 0 |
| Media | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-bb425f03-document-ocr cargo test -p compass-media --locked` | exit 0 |
| Core/semantic/history | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-bb425f03-document-ocr cargo test -p compass-core -p compass-semantic -p compass-history --locked` | exit 0 |
| CLI contract | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-bb425f03-document-ocr cargo test -p compass-cli --test compass_product --locked` | exit 0 |
| OCR qualification | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-bb425f03-document-ocr ./scripts/qualify_document_ocr_v1.sh --fixtures-only` | exit 0; all required profiles/cases pass |
| Base documents | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-bb425f03-document-ocr ./scripts/qualify_document_graph_v1.sh --fixtures-only` | exit 0 without OCR/model installation |
| Lint | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-bb425f03-document-ocr cargo clippy -p compass-ocr -p compass-media -p compass-core -p compass-semantic -p compass-history -p compass-cli --all-targets --all-features --locked -- -D warnings` | exit 0 |
| Format | `cargo fmt --all -- --check` | exit 0 |
| Boundary | `sh scripts/check_product_boundary.sh` | exit 0 |

If `/Volumes/Workspace` is unavailable or unwritable, stop. Never fall back to
the checkout's `target/` directory and never reuse another worktree's target.

## Scope

**In scope**:

- `crates/compass-ocr/Cargo.toml` and `crates/compass-ocr/src/` (create)
- `crates/compass-ocr/tests/` (create)
- `crates/compass-media/src/document.rs`, `limits.rs`, and focused PDF/OOXML
  raster-candidate/fusion modules produced by Plans 007 and 010
- `crates/compass-media/tests/document_ocr.rs` (create)
- focused `compass-files`, `compass-core`, `compass-semantic`, and
  `compass-history` option/fingerprint/cache/publication changes and tests
- `crates/compass-cli/src/document_commands.rs` and
  `crates/compass-cli/src/model_commands.rs` (create), thin dispatch/help wiring,
  and subprocess-style CLI tests
- root `Cargo.toml` and `Cargo.lock` for reviewed dependencies/workspace wiring
- `tests/qualification/document-ocr/v1/` (create)
- `scripts/qualify_document_ocr_v1.sh` and deterministic fixture/metric helpers
- `docs/design/document-processing.md`,
  `docs/design/security-and-privacy.md`,
  `docs/implementation/document-ocr-qualification.md` (create),
  `docs/reference/document-formats.md`, `docs/reference/commands.md`,
  `docs/README.md`, `PERFORMANCE.md`, `COMPATIBILITY.md`, `CHANGELOG.md`, and
  `MIGRATION.md` only if users must take action
- `.github/workflows/compass-ci.yml` and `Makefile` only after the fixture-only
  gate is stable and credential/network free

**Out of scope**:

- Legacy `.doc`, `.xls`, or `.ppt` conversion.
- Cloud OCR APIs or semantic-provider vision calls.
- Generative VLM publication of tables, formulas, charts, or relationships.
- Handwriting support unless the selected model is independently qualified and
  documented; do not imply it from scene-text examples.
- Pixel-perfect Office rendering or OCR of whole Office pages.
- Formula/macro/field/OLE/ActiveX execution.
- Fetching external OOXML relationships or PDF resources.
- Automatically downloading models during extract, update, watch, history,
  MCP, or document inspection.
- Committing model weights, opaque private documents, generated graphs, OCR
  caches, or temporary raster images.
- Adding multiple production engines “for choice” before each passes the same
  corpus and maintenance cost is justified.

## Git workflow

- Suggested branch: `advisor/022-quality-document-ocr`.
- Use logical commits: qualification spike; contracts; raster candidates;
  engine/model delivery; fusion; orchestration/history; CLI; qualification/docs.
- Use conventional messages such as `feat(ocr): add versioned observation contract`.
- Do not push or open a PR unless instructed.

## Phases

### Phase 0: Freeze the baseline and select a viable engine/renderer

Before production edits:

1. Finish Plans 006–008 and 010 or reconcile their live equivalents.
2. Create a small, license-safe spike corpus and metric tool outside production
   routing but inside the planned qualification tree.
3. Run PP-OCRv6 small/medium, Tesseract 5 `tessdata_best`, and `ocrs`/RTen with
   pinned versions/models/configuration. Record install/runtime footprint,
   license, supported scripts, CER/WER, geometry, CPU time, memory, and output
   schema stability.
4. Test whether a reviewed pure-Rust runtime can execute the selected
   PP-OCRv6 export without unsupported operators or quality drift. Do not
   convert model weights in an unreviewable ad hoc script; pin the converter,
   source revision, command, output size, and digest.
5. Qualify the live `oxidize-pdf` renderer or an approved pure-Rust upgrade on
   scanned, rotated, image-only, font, transparency, clipping, and malformed
   PDF fixtures.
6. Write the measured selection decision into
   `docs/design/document-processing.md`. Name one production OCR backend and
   one renderer. Keep other engines qualification-only.

**Verify**: a checked machine-readable report contains every candidate,
version, model digest, corpus version, metric, platform, and rejection reason;
rerunning it with installed artifacts requires no network.

### Phase 1: Add the engine-neutral OCR crate and validation contract

Create `compass-ocr` with the types and validators above. Add traits equivalent
to:

```rust
pub trait OcrEngine {
    fn identity(&self) -> &OcrProfileIdentity;
    fn recognize(
        &self,
        requests: &[PreparedOcrRequest],
        limits: &OcrLimits,
        cancellation: &AtomicBool,
    ) -> Result<Vec<OcrResult>, OcrError>;
}
```

The trait receives already-decoded, bounded rasters and never paths into the
source corpus. Add typed errors for unsupported profile/language, model absent
or invalid, request rejected, inference failure, timeout/cancel, protocol
failure, and output validation.

Write contract tests for unknown major, invalid geometry, duplicate/missing
request IDs, noncanonical language order, low/high confidence bounds, text and
region ceilings, deterministic serialization, and cancellation.

**Verify**: `cargo test -p compass-ocr --locked` exits 0 without models,
network, Python, or helper executables.

### Phase 2: Extract bounded raster candidates without invoking OCR

In `compass-media`, add typed `RasterCandidate` values for:

- PDF page raster requests with page locator, dimensions, rotation, native-text
  coverage signals, and selection reason;
- DOCX/PPTX/XLSX internal image parts with owning block/shape/sheet locator,
  relationship kind, media type, byte digest, declared dimensions when known,
  and bounded bytes.

Use the Plan-010 OPC reader for all image parts. Reject duplicate/escaping
parts and external targets. Deduplicate decode work by byte digest while
preserving every owning occurrence. Candidate discovery itself must not decode
unbounded pixels or load a model.

Add positive and hostile synthetic fixtures. Assert no candidate includes an
absolute path, external URL bytes, macro/OLE content, or uncontrolled package
metadata.

**Verify**: media tests exit 0; OCR mode `off` produces byte-for-byte equivalent
native artifacts and never constructs an engine.

### Phase 3: Implement deterministic rasterization and preprocessing

Implement the selected PDF renderer behind `PageRasterizer`. Normalize direct
and embedded images through one bounded pipeline. Calculate all allocation
sizes before decoding; apply EXIF orientation, alpha compositing, resizing, and
tiling exactly as specified. Record rasterizer/preprocessing identity in every
request.

Tests must cover 1×1, exact-limit, one-over-limit, huge declared dimensions,
truncated streams, animated/multi-frame inputs, orientation, alpha, long-edge
resize, tile overlap, tile-coordinate reassembly, and cancellation. Compare
small rendered PDF fixtures against checked pixel/geometry expectations, not a
human screenshot.

**Verify**: media and OCR tests exit 0; peak raster allocations remain within
declared bounds under the limit corpus; no test invokes system PDF tools.

### Phase 4: Implement pinned model installation and the selected engine

Add a static model/profile manifest containing source repository, immutable
revision, artifact filenames, exact sizes, SHA-256 digests, license/model-card
metadata, supported language tags, and preprocessing requirements. Follow the
verified temporary-write pattern in `compass-transcribe/src/models.rs`, but
keep OCR ownership in `compass-ocr`.

If using an in-process engine, disable implicit network/model discovery and
bound engine threads. If using a helper, implement only the Compass protocol
above; include handshake identity and validate every response. The helper must
be killable on timeout/cancel and must not leave descendants running.

Tests use fixture fetchers and fake helpers for missing/short/long/bad-digest
artifacts, stale markers, redirects/host rejection, nonzero exit, timeout,
oversized output, malformed JSON, wrong request/model identity, missing/extra
results, bad geometry, stderr redaction, cleanup, and offline replay.

**Verify**: OCR tests exit 0 offline; a native acceptance test is opt-in and
skips only when the exact verified model is absent, never because a network
download failed during ordinary tests.

### Phase 5: Fuse OCR observations into document artifacts

Extend `DocumentArtifact` with OCR origin, locators, profile identity, and
visual coverage. Implement native/OCR comparison and fusion exactly as defined
above. Preserve native order; insert OCR children under the owning page/image
block in deterministic geometric order. Repeated equal OCR text remains
separate when geometry/source occurrence differs.

Add tests for corroboration, conflict, native geometry absent, low confidence,
RTL/unknown reading order, tiled overlap deduplication, repeated embedded image
bytes at different locations, and OCR requested with one failed candidate.

Increment `DOCUMENT_NORMALIZER_VERSION`. Add unknown-version and old-cache-miss
tests; do not migrate or reinterpret cached flattened strings.

**Verify**: media contract tests and document qualification pass; born-digital
fixtures gain no duplicate semantic text under `auto`.

### Phase 6: Integrate core, semantic packing, cache, and history

Add `DocumentProcessingOptions` at the application boundary and pass an OCR
engine explicitly. Prepare each rich document once, cache validated OCR results
under the complete fingerprint, then derive both structural graph blocks and
semantic slices from the same artifact. `compass-semantic` must never rerun OCR.

Cache behavior:

- source or profile changes miss;
- repeated image bytes reuse observations but retain distinct source locators;
- partial/failed OCR is not finalized as complete;
- cache corruption is rejected explicitly;
- `off` cache entries never satisfy `auto`/`always`;
- an OCR cache cannot be reused under a different renderer, preprocessing,
  language, or model digest.

History tests cover offline materialization, missing pinned model, exact profile
replay, immutable realization, and profile mismatch before diff. If exact OCR
reproduction differs across supported CPU architectures, publish it only as
derived semantic evidence and preserve the exact stored realization.

**Verify**: core/semantic/history tests exit 0; fake-engine call counts prove
one preparation per changed document and zero calls on a valid warm build.

### Phase 7: Add inspect, extraction, and model-management UX

Implement the CLI contract above with reusable domain calls. Add help, examples,
mutual exclusions, defaults, text/JSON schemas, stdout/stderr behavior, and exit
codes. `document inspect` is read-only unless `--output` is explicitly added in
a later approved contract. Model installation is atomic and reports exact
profile identity; extraction never prompts or downloads.

CLI tests execute the binary and assert:

- OCR-off inspection works with no model/helper;
- explicit OCR with missing model fails with one actionable install command;
- `auto` scans only eligible candidates;
- `always` obeys caps;
- JSON is valid `compass.document.inspect/1` with no absolute temporary paths;
- corrupt/timeout/protocol errors use stderr and nonzero status;
- `--allow-partial` is visible in JSON/text and never reports complete;
- noninteractive commands never prompt, open, or fetch.

**Verify**: CLI product and focused document tests exit 0; command reference
examples match `--help` exactly.

### Phase 8: Build the OCR qualification and regression gate

Promote the spike corpus to `document-ocr/v1`, add typed manifest validation,
metric computation, hostile cases, fake-engine contract cases, and opt-in exact
model acceptance. Create `scripts/qualify_document_ocr_v1.sh` modeled after the
document/code qualification scripts. It must reject unknown flags, require an
external Cargo target, download nothing, and emit bounded machine-readable
metrics plus a concise summary.

Run candidate/reference acceptance outside normal CI only when pinned models
are provisioned. Normal CI must always run contract, fake-engine, limits, and a
small redistributable model/fixture path if licensing allows. Base document
qualification continues to pass on a machine with no OCR model.

**Verify**: every release gate in “OCR qualification contract” is
machine-checked or explicitly reported as a blocking unmeasured item; changing
one ground-truth string, digest, limit, or model identity makes the gate fail.

### Phase 9: Publish documentation, compatibility, and performance evidence

Update design, security, format, command, qualification, performance,
compatibility, and changelog documents. State:

- OCR is optional, local, model-backed, and off by default;
- which exact scripts/languages/input classes are qualified;
- native text precedence and OCR conflict behavior;
- installed model footprint and source/license;
- network behavior for installation versus extraction;
- all important limits and partial semantics;
- PDF renderer and unsupported constructs;
- reproducible accuracy/performance commands and results;
- historical fingerprint/replay behavior.

Add `MIGRATION.md` only if an existing public default/schema requires user
action. Never claim support for a script, handwriting, table reconstruction, or
platform absent from the corpus.

Run all commands in the table, the native baseline required by root `AGENTS.md`,
and `git diff --check`. Inspect status for model weights, temporary images,
caches, generated graphs, and unrelated edits; none may be committed.

## Test plan

- OCR schema, serde, validation, canonical order, geometry, limits, and cancel.
- Model manifest/download verification with only local fixture fetchers.
- Helper lifecycle/protocol/timeout/output/redaction/cleanup when applicable.
- PDF page selection/rasterization and malformed/limit behavior.
- DOCX/PPTX/XLSX embedded-image discovery, ownership, repeat-digest reuse, and
  external/macro/object negatives.
- Preprocessing orientation/alpha/resize/tile/reassembly determinism.
- Native preference, corroboration, conflict, low confidence, geometry absence,
  reading-order limitations, and no text-based identities.
- Cold/warm/change/profile/cache-corruption and partial-completion behavior.
- Historical offline/profile/immutable-realization behavior.
- CLI help, inspect JSON/text, missing model, no implicit network, and exit code.
- Cross-format accuracy, geometry, duplicate, performance, architecture, and
  hostile qualification corpus.
- Base document and code graph gates remain unchanged and credential-free.

## Done criteria

- [ ] Plans 006–008 and 010 are complete and no document path still flattens
  media before the artifact boundary.
- [ ] One OCR backend and one PDF renderer are selected by measured Compass
  evidence; all rejected candidates and trade-offs are documented.
- [ ] Native document processing works identically with OCR absent/off.
- [ ] Explicit OCR is local during extraction, bounded, cancelable, versioned,
  and provenance-preserving.
- [ ] No model is downloaded implicitly and every installed artifact is pinned
  by immutable revision, exact size, SHA-256, and license metadata.
- [ ] Native text is never replaced; corroboration/conflicts are explicit.
- [ ] PDF and OOXML OCR blocks retain exact owning logical locators and bounded
  pixel geometry.
- [ ] Cache/history fingerprints include every meaning-affecting OCR input and
  never fall back across profiles.
- [ ] Partial/failed OCR never publishes or caches as complete.
- [ ] `compass document inspect`, extraction flags, and model commands have
  tested text/JSON/help/exit contracts.
- [ ] OCR qualification passes its accuracy, duplicate, safety, determinism,
  and resource gates on every claimed script/platform.
- [ ] Base document qualification passes without OCR models or helper tools.
- [ ] Targeted tests, workspace baseline, lint, format, boundary, and diff checks
  pass with the required external target.
- [ ] Plan 022 is marked `DONE` in `advisor-plans/README.md`.

## STOP conditions

Stop and report instead of improvising if:

- any prerequisite plan is incomplete or its artifact/locator/cache contract is
  incompatible with this design;
- the selected model's weights, datasets, or redistribution license cannot be
  verified for Compass's distribution;
- PP-OCR model conversion/inference requires unpinned tooling or changes quality
  beyond the declared gate;
- no renderer can produce bounded, trustworthy PDF pixels without adding an
  unapproved native/runtime dependency;
- a candidate engine requires network access during extraction;
- OCR output can only be integrated by replacing native text or fabricating
  package/PDF byte offsets;
- cross-platform recognition/geometry cannot meet the required semantic
  equivalence; keep it out of structural publication and request review;
- a helper cannot be reliably timed out, killed, output-bounded, and cleaned up;
- arbitrary user executables, shell strings, external OOXML links, formulas,
  macros, OLE, or document scripts would need to execute;
- a public schema major, stable ID, or default must change without compatibility
  and migration approval;
- qualification needs private documents, credentials, a cloud OCR API, or
  network access;
- an in-scope file overlaps user edits that cannot be preserved;
- `/Volumes/Workspace` is unavailable or a Cargo command would use local
  `target/`.

## Maintenance notes

OCR quality changes whenever the engine, model, renderer, preprocessing,
language policy, or fusion threshold changes. Treat each as a normalizer/profile
change with cache/history review and corpus evidence. Keep the base document
gate independent so optional OCR cannot become an accidental product dependency.

Reviewers should scrutinize source authority, geometry, duplicate suppression,
model provenance, temp-file sensitivity, helper containment, cross-platform
drift, and false completeness. High average OCR accuracy does not excuse one
unbounded path or an invented native-text replacement.

## Primary references for engine evaluation

- [`ocrs` project](https://github.com/robertknight/ocrs) — end-to-end Rust OCR
  goals, RTen inference, current early-preview status, and Latin-only support.
- [RTen project](https://github.com/robertknight/rten) — end-to-end Rust,
  CPU-only ONNX/RTen inference and supported deployment targets.
- [PaddleOCR project](https://github.com/PaddlePaddle/PaddleOCR) and
  [official OCR pipeline documentation](https://github.com/PaddlePaddle/PaddleOCR/blob/main/docs/version3.x/pipeline_usage/OCR.en.md)
  — PP-OCRv6 profiles, multilingual coverage, orientation/unwarping, model
  distribution, and reported quality/performance.
- [Tesseract 5 user manual](https://tesseract-ocr.github.io/tessdoc/) — stable
  classical baseline, official trained-data families, languages, and license.
- [`oxidize-pdf` API documentation](https://docs.rs/oxidize-pdf/latest/oxidize_pdf/)
  — current pure-Rust parsing/rendering/OCR integration claims that must be
  verified against Compass's pinned version and qualification corpus.
