# Document format reference

This page records the current deterministic document boundaries. A file can be
discoverable without having a structural extractor; consumers should inspect
the graph producer and diagnostics rather than infer support from an extension
alone.

## Markdown

| Construct | Current behavior | Provenance |
| --- | --- | --- |
| ATX / Setext headings | Heading node, hierarchy, source-order duplicate slug, optional explicit ID | exact node range |
| Paragraphs and inline text | Section-qualified paragraph block | exact node range |
| Lists and task items | List/list-item blocks; `task_checked` when present | `contains` + exact ranges |
| Block quotes and thematic breaks | Structural block | exact node range |
| Fenced / indented code | Code block; fenced info string becomes `language` | exact node range |
| Pipe tables | Table, header, row, and cell blocks with header-qualified labels | nested `contains` edges and exact cell-owned references |
| Reference definitions | Definition block and definition relationship | definition range |
| Inline/reference/autolinks | Link relationship with `link_kind` | link-site range |
| Footnotes | Bounded definition nodes and reference relationships | exact definition/reference ranges |
| Wikilinks | Local link evidence with `link_kind: "wikilink"` | link-site range |
| MDX / Quarto extensions | Bounded `other` blocks; no execution | exact source range |
| Images | Ignored as document relationships | no fetch or edge |
| Frontmatter | Bounded `document_metadata` map | root metadata; body offsets unchanged |
| Malformed syntax | Recovered evidence plus bounded diagnostic | extraction quality extension |

Markdown is parsed from the caller-supplied bytes with statically linked
Tree-sitter block and inline grammars. Supported source extensions are
`.md`, `.markdown`, `.mdx`, `.qmd`, and `.skill`.

### Stable fields

Document and block nodes retain the common graph fields `id`, `label`,
`file_type`, `document_kind`, `source_file`, `_origin`, `start_byte`,
`end_byte`, `start_line`, `end_line`, `column_start`, and `column_end` where
applicable. Structural blocks also carry a deterministic `qualified_name` and,
when nested under a heading, `document_section`. Markdown-specific root
extensions include:

- `markdown_block_count`;
- `markdown_link_count`;
- `markdown_diagnostics`;
- `markdown_unresolved_links`;
- `markdown_external_links`.
- `markdown_footnote_count` and `markdown_other_count`.

These fields are extensible graph attributes. Consumers must preserve unknown
attributes and must not parse stable IDs as path components.

### Frontmatter limits

- opening and closing delimiters must be whole lines within 64 KiB;
- at most 256 metadata keys and 256 scalar-array items are published;
- individual metadata keys and strings are capped at 16 KiB;
- nested mappings, YAML tags/aliases, and non-scalar arrays are diagnosed and
  omitted rather than projected as arbitrary graph data.

### Link boundary

External links are recorded as evidence but never fetched. Same-file and
cross-file fragments are percent-decoded within a fixed bound and resolve only
to a unique heading slug or explicit ID.
Project resolution supports exact paths, `.md`/`.markdown`/`.mdx`/`.qmd`/`.skill`
extension inference, repository-root paths, directory `README`/`index`
documents, and unique wikilink stems. It can also connect documentation to the
file-inventory node for any extracted source language. Ambiguous or missing
targets and fragments remain unresolved; Compass does not select the first
candidate, fall back to a document root, or invent a node.

## HTML

| Construct | Current behavior | Provenance |
| --- | --- | --- |
| Title and headings | Title metadata/node; `h1`–`h6` heading nodes with levels | exact element range |
| `main`/`article`/`section`/`nav` | Landmark nodes in source order | exact element range |
| Paragraphs, lists/items, block quotes | Semantic nodes and `contains` edges | exact element range |
| `pre`/`code` | Preformatted/code nodes; visible text excludes scripts | exact element range |
| Tables, rows, cells | Table hierarchy including `th`/`td` | exact element range |
| Anchors and resource links | `href`/`rel` attributes plus local/external evidence | link element range |
| `meta`, canonical, base | Bounded root metadata and link evidence | element/link range |
| Entities and whitespace | Decoded visible text; block order preserved | source-backed node ranges |
| `script`, `style`, `template`, `noscript` | Entire subtree skipped | no graph node or link |
| Malformed markup | Tree-sitter recovery plus bounded diagnostics | recovery/root range |

HTML uses the exact pinned `tree-sitter-html` binding and the source-driven
`Engine::extract_source` API. URL ingestion calls the same
`compass_languages::normalize_html` renderer. Relative URLs are resolved
lexically against the validated source/base URL, never fetched.

## Discovery versus structural extraction

| Format | Discovery classification | Structural extractor in this release |
| --- | --- | --- |
| HTML / HTM | document | structural Tree-sitter adapter and shared ingestion renderer |
| PDF | document | native page/text blocks; optional page OCR |
| DOCX | document | ordered paragraphs, headings, lists, tables, notes, links, embedded-image OCR |
| PPTX | document | relationship-ordered slides, shapes, tables, notes, links, embedded-image OCR |
| RTF | not a general local document adapter | not yet |
| XLSX | document | sparse typed sheets/rows/cells, formulas as inert metadata, embedded-image OCR |
| TXT / RST | document | generic/document fallback only |

PDF and Office adapters emit `compass.document/1`. Locators are typed as PDF
page/item, OOXML package part/path, slide/shape, sheet/row/column, or OCR owner
plus pixel polygon. Structural graph nodes preserve the serialized locator and
document origin. Unknown schema or normalizer versions fail rather than being
flattened through a compatibility adapter.

### OCR commands and defaults

```text
compass document inspect FILE --ocr off|auto|always --format text|json
compass extract PATH --ocr off|auto|always
compass models list|install|verify
```

OCR defaults to `off`. Native extraction needs no model. `auto` and `always`
require a verified local profile and never download implicitly. Use
`--ocr-language TAG` repeatedly for bounded language hints and
`--allow-partial` only when incomplete visual coverage is acceptable. JSON
inspection uses `compass.document.inspect/1` and includes the policy, artifact,
limits, diagnostics, visual coverage, and exact OCR profile identity.

Managed OCR is unavailable on Intel (`x86_64`) macOS because its pinned ONNX
runtime has no self-contained distribution for that target. Native extraction
and `--ocr off` remain available without additional installation. Compass
rejects model installation and OCR-enabled processing there before any model
download.

Document and cache files are capped while streaming, so size checks still hold
if an input changes during a read. PDF rasterization reserves aggregate pixels
before rendering each page, and tiled recognition checks the document deadline
before and after each inference unit. One native inference call cannot be
preempted midway; its result is discarded as a timeout if the deadline has
elapsed. Concurrent installation of the same model profile waits on a bounded
lock, and verification rejects symlinked artifacts or markers.

## Related contracts

- [Configuration](configuration.md)
- [Output reference](outputs.md)
- [Compatibility](../../COMPATIBILITY.md)
