---
meta:
  contentType: Conceptual
  title: Structural document processing
  navLabel: Document Processing
  category: Design
  overview: How Compass turns Markdown and HTML bytes into bounded, deterministic graph evidence.
  goal: Define the ownership, provenance, and cache rules for local text documents.
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
extracted strings. The current structural implementation is Markdown-first:
the same bounded bytes read by the build pipeline are parsed into a document
root, structural blocks, and provenance-preserving relationships.

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
and an exact byte range. Heading nodes additionally carry `heading_level`,
`heading_style`, `qualified_name`, and a deterministic `anchor_slug`; headings
with `{#explicit-id}` retain `explicit_id`.

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
without fetching them. Supported local links produce `references` edges, while
links to an existing source document may use the `documents` relation. A
fragment-only link resolves to a heading only when its slug or explicit ID is
unique. Duplicate or missing fragments remain in `markdown_unresolved_links`
with an explicit reason; extraction never selects the first same-named heading.

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

File discovery recognizes several document extensions, but recognition is not
the same as structural extraction. DOCX and XLSX retain their bounded media
conversion surfaces; PPTX and RTF remain future format adapters. See the
[document format reference](../reference/document-formats.md) for the current
matrix. Markdown and HTML links may point at those formats, but Compass does
not fetch or execute a linked resource during extraction.

## Related pages

- [Document format reference](../reference/document-formats.md)
- [Language architecture](language-architecture.md)
- [Extraction pipeline](../implementation/extraction-pipeline.md)
- [Graph model](../concepts/graph-model.md)
