---
meta:
  contentType: Conceptual
  title: Structural document processing
  navLabel: Document Processing
  category: Design
  overview: How Compass turns text, PDF, and Office bytes into bounded native and OCR evidence.
  goal: Define ownership, provenance, cache, OCR, and graph rules for local documents.
  audience:
    - Compass contributors
    - technical evaluators
  contentPlan:
    - source-driven extraction
    - Markdown and HTML structure and metadata
    - links and ambiguity
    - cache and security boundaries
  openQuestions: []
---

# Structural document processing

Compass treats a document as an ordered source artifact, not as a bag of
extracted strings. Markdown and HTML retain exact source ranges. PDF, DOCX,
PPTX, and XLSX use the versioned `compass.document/1` intermediate artifact,
typed logical locators, and one shared projection into graph blocks and
semantic slices.

## Ownership and data flow

```text
bounded source bytes
        |
        v
compass-languages::Engine
        |
        +--> pinned Markdown block/inline grammars
        +--> pinned HTML grammar and shared renderer
        +--> bounded frontmatter/entity/URL decoders
        |
        v
document root + structural blocks + link evidence
        |
        v
compass-resolve project inventory + heading targets
        |
        v
compass-graph / compass-core publication
```

`Engine::extract_source` and the combined extraction path pass the already-read
bytes to the Markdown producer. This avoids a second file read and preserves
the caller's source-file identity for cache, repository-relative paths, and
source ranges. The standalone compatibility path still accepts a `Path` and
reads it once.

The Markdown grammars are statically linked through the pinned `tree-sitter-md`
crate and HTML uses the exact pinned `tree-sitter-html` crate. The vendored
language pack remains the owner for the general language registry; the direct
HTML binding is deliberately parser-only because this release's pack build
does not expose an HTML static loader. Neither path downloads a grammar at
runtime, invokes Python, calls a model, or follows a URL.

## Markdown projection

Every file has one root node with:

- `document_format: "markdown"` and `document_kind: "document"`;
- the source file and exact whole-document byte/line range;
- deterministic `document_metadata` when bounded frontmatter is valid.

The structural projection emits ordered nodes for headings, paragraphs, lists
and list items, block quotes, thematic breaks, fenced and indented code,
pipe-table containers/headers/rows/cells, HTML blocks, and reference
definitions. Each node carries `document_kind`, `block_index`, source identity,
a section-qualified `qualified_name`, and an exact byte range. Blocks beneath a
heading also carry `document_section`. Heading nodes additionally carry
`heading_level`, `heading_style`, and a deterministic `anchor_slug`; repeated
automatic slugs receive source-order `-1`, `-2`, … suffixes, while headings
with `{#explicit-id}` retain `explicit_id` and duplicate explicit IDs remain
ambiguous.

Nested blocks are represented by `contains` edges. Inline links are owned by
the smallest containing structural block, so a link in a list item or table
cell does not get attributed to the surrounding list or table as well. Parser
helper nodes such as markers, destinations, and continuation tokens are never
published as blocks.

## Frontmatter policy

Frontmatter is recognized only when the source begins with a whole-line `---`
(an optional UTF-8 BOM is accepted) and a whole-line closing delimiter appears
within 64 KiB. It is parsed with the workspace YAML implementation and only
JSON-compatible scalars and bounded scalar arrays are published. Mappings,
aliases, tags, oversized values, and arrays containing non-scalars produce a
bounded diagnostic and do not become graph attributes. Keys are deterministic
and capped at 256 entries; individual strings and arrays are bounded.

Frontmatter is metadata, not visible Markdown body text. Body node ranges still
point into the original bytes, including CRLF and non-UTF-8 input (labels use a
lossy display representation while offsets remain byte offsets).

## Link resolution

Compass records external links as bounded `markdown_external_links` evidence
without fetching them. The per-file extractor preserves local link spelling
and its exact source site; the project resolver selects a target only against
the complete extracted inventory. Supported local links produce `references`
edges, while links to source code use `documents` and target that language's
file-inventory node.

Exact and repository-root paths, `.md`/`.markdown`/`.mdx`/`.qmd`/`.skill` extension
inference, directory `README`/`index` documents, and unique wikilink stems are
bounded resolution rules. A same-file or cross-file fragment is percent-decoded
within a fixed bound and resolves only when its slug or explicit ID is unique.
Duplicate or missing fragments and
ambiguous extension or stem candidates remain unresolved; resolution never
selects the first candidate or substitutes a document root. Exact resolved
relationships retain source-backed confidence and are therefore available at
the default low inference level.

Reference definitions and usages retain separate source sites. Wikilinks,
autolinks, email links, inline links, and reference links carry a `link_kind`,
line, byte range, source file, and extracted confidence. Images are not treated
as document relationships, and links inside fenced code are inert.

Footnote definitions and references are emitted as bounded
`footnote_definition` nodes and `link_kind: "footnote"` relationships. Duplicate
or missing footnote labels remain explicit unresolved evidence. MDX imports,
JSX-like components, expressions, and Quarto directives are represented as
bounded `other` blocks with an `other_kind` and `source_syntax` instead of being
executed or silently discarded.

## HTML projection and normalization

HTML files (`.html` and `.htm`) use the same source-driven API as Markdown. The
root has `document_format: "html"`; semantic nodes cover headings, paragraphs,
lists/items, block quotes, preformatted/code blocks, tables/rows/cells,
landmarks (`main`, `article`, `section`, and `nav`), anchors, title, metadata,
resource links, and base URLs. Every node and relationship keeps an exact byte,
line, and column range into the original HTML bytes.

The HTML adapter decodes numeric and common named entities, preserves document
order, and records bounded `html_title`, `html_meta`, `html_canonical`,
`html_base_href`, and `html_visible_text` metadata. `script`, `style`,
`template`, and `noscript` subtrees are never published. Anchor and resource
links carry their `href`/`rel` provenance; same-file fragments resolve only to a
unique `id`/`name`, while external and unsupported links remain evidence with
an explicit reason. Relative links are resolved against a validated HTTP(S)
source/base URL or a lexically normalized local path. Compass never fetches a
discovered URL.

`compass-ingest` calls `compass_languages::normalize_html` rather than using a
regular-expression tag stripper. The renderer emits conservative Markdown
block boundaries and links, so fetched webpages and local HTML files share the
same visibility, entity, whitespace, and security rules.

## Cache and trust boundaries

Markdown file hashes include the raw frontmatter bytes. A metadata-only edit
therefore invalidates structural and semantic cache entries just like a body
edit. The legacy `body_content` helper remains available for callers that
explicitly need the old body-only view; it is not used as the graph cache key.

All parser work is bounded by the caller's source-read limits plus explicit
frontmatter, block, link, metadata, and diagnostic caps. Inputs are never
executed, fetched, or interpreted as configuration. Unknown graph attributes
remain extensible, and consumers should preserve them.

## Structural and semantic coexistence

Semantic refreshes are additive for Markdown and HTML. When a provider returns
an updated concept for one of these files, Compass retains the byte-identical
document, block, containment, and link subgraph produced by the local adapter;
provider concepts and edges are published alongside it. A provider failure or
partial response cannot replace a deterministic structural realization.

## Other document formats

PDF and OOXML packages are decoded in pure Rust under centralized raw-byte,
archive-member, expansion-ratio, XML-depth, block, link, row, cell, page, and
raster limits. DOCX body order, PPTX relationship slide order, and sparse XLSX
coordinates are preserved. Spreadsheet formulas are evidence and are never
executed. External OOXML relationships remain inert.

OCR is an optional derived layer and is off by default. `auto` selects PDF
pages with little native text and eligible embedded Office images; `always`
selects every bounded candidate. Native text remains authoritative and OCR
observations retain the owning page/image locator, polygon, confidence, exact
engine version, profile, model digests, and preprocessing version. OCR never
replaces or silently deduplicates native blocks.

Preprocessing version 2 applies declared EXIF orientation exactly once,
composites alpha onto white, resizes with the fixed triangle filter, and tiles
rasters above the 2,048-pixel engine side with 128 pixels of overlap. Tile
regions are mapped back to the normalized source raster and equal overlapping
regions are deduplicated deterministically. Decoding, PDF page rendering,
candidate iteration, and inference boundaries honor cancellation; the document
deadline is 600 seconds. The in-process runtime uses one inter-op and one
intra-op thread.

`compass-core` prepares each rich document once. Complete artifacts are cached
atomically by source SHA-256 plus document schema, normalizer, renderer, OCR
policy, profile manifest, preprocessing version, and language hints. Corrupt
or incompatible entries fail explicitly; partial OCR is never finalized as a
complete cache entry. The same prepared artifact feeds structural publication
and gap-free Unicode-safe semantic slices. The semantic layer does not load an
OCR engine or maintain a second document cache.

The PP-OCRv6 runtime is compiled with Compass. Users install no Python,
Tesseract, office suite, Poppler, Java, or system ONNX package. Model weights
are deliberately separate: `compass models install pp-ocrv6-small` is the only
download path and validates a fixed allowlisted HTTPS source, declared size,
SHA-256, and atomic verified marker. Inspection and extraction never download
or prompt.

The production engine identity is OAR-OCR 0.9.2 with its in-process ONNX
backend. The model source is the immutable GreatV/OAR-OCR `v0.7.0` GitHub
release: the small profile is 31,114,837 bytes and the medium profile is
138,662,763 bytes across detector, recognizer, and shared dictionary. Each
artifact has a compiled SHA-256. Hayro 0.7.1 is the sole PDF renderer and is
fingerprinted as `hayro/0.7.1@300dpi`. Neither boundary invokes a system tool.

The checked installed-model smoke gate currently establishes only clean,
synthetic English recognition and runtime availability; it recorded 0 CER for
`COMPASS OCR 2026` on the development aarch64 macOS host. This is not a broad
multilingual, photographed-document, handwriting, or comparative quality
claim. Release promotion for additional input classes and architectures must
come from the corpus and gates in the qualification guide.

## Related pages

- [Document format reference](../reference/document-formats.md)
- [Language architecture](language-architecture.md)
- [Extraction pipeline](../implementation/extraction-pipeline.md)
- [Document OCR qualification](../implementation/document-ocr-qualification.md)
- [Graph model](../concepts/graph-model.md)
