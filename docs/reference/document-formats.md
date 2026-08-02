# Document format reference

This page records the current deterministic document boundaries. A file can be
discoverable without having a structural extractor; consumers should inspect
the graph producer and diagnostics rather than infer support from an extension
alone.

## Markdown

| Construct | Current behavior | Provenance |
| --- | --- | --- |
| ATX / Setext headings | Heading node, hierarchy, slug, optional explicit ID | exact node range |
| Paragraphs and inline text | Paragraph block | exact node range |
| Lists and task items | List/list-item blocks; `task_checked` when present | `contains` + exact ranges |
| Block quotes and thematic breaks | Structural block | exact node range |
| Fenced / indented code | Code block; fenced info string becomes `language` | exact node range |
| Pipe tables | Table, header, row, and cell blocks | nested `contains` edges |
| Reference definitions | Definition block and definition relationship | definition range |
| Inline/reference/autolinks | Link relationship with `link_kind` | link-site range |
| Wikilinks | Local link evidence with `link_kind: "wikilink"` | link-site range |
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
applicable. Markdown-specific root extensions include:

- `markdown_block_count`;
- `markdown_link_count`;
- `markdown_diagnostics`;
- `markdown_unresolved_links`;
- `markdown_external_links`.

These fields are extensible graph attributes. Consumers must preserve unknown
attributes and must not parse stable IDs as path components.

### Frontmatter limits

- opening and closing delimiters must be whole lines within 64 KiB;
- at most 256 metadata keys and 256 scalar-array items are published;
- individual metadata keys and strings are capped at 16 KiB;
- nested mappings, YAML tags/aliases, and non-scalar arrays are diagnosed and
  omitted rather than projected as arbitrary graph data.

### Link boundary

External links are recorded as evidence but never fetched. Same-file fragments
resolve only to a unique heading slug or explicit ID. Ambiguous and missing
fragments are explicit unresolved evidence. Unsupported local suffixes and
missing document targets do not create invented nodes.

## Discovery versus structural extraction

| Format | Discovery classification | Structural extractor in this release |
| --- | --- | --- |
| HTML / HTM | document | not yet a local language adapter |
| DOCX | document/media | media conversion surface; no native block graph |
| PPTX | not a general local document adapter | not yet |
| RTF | not a general local document adapter | not yet |
| XLSX | document/media | media conversion surface; no native block graph |
| TXT / RST | document | generic/document fallback only |

This distinction keeps product claims honest: future HTML, office, and rich
text work must add bounded parsing, exact or explicitly normalized locators,
security tests, and cache/version contracts before it becomes graph evidence.

## Related contracts

- [Structural document processing](../design/document-processing.md)
- [Output reference](outputs.md)
- [Compatibility](../../COMPATIBILITY.md)
