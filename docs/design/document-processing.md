---
meta:
  contentType: Conceptual
  title: Structural document processing
  navLabel: Document Processing
  category: Design
  overview: How Compass turns Markdown bytes into bounded, deterministic graph evidence.
  goal: Define the ownership, provenance, and cache rules for local text documents.
  audience:
    - Compass contributors
    - technical evaluators
  contentPlan:
    - source-driven extraction
    - Markdown structure and metadata
    - links and ambiguity
    - cache and security boundaries
  openQuestions: []
---

# Structural document processing

Compass treats a document as an ordered source artifact, not as a bag of
extracted strings. The current structural implementation is Markdown-first:
the same bounded bytes read by the build pipeline are parsed into a document
root, structural blocks, and provenance-preserving relationships.

> **Who this page is for:** contributors and integrators who need to understand
> document graph facts.
>
> **You will learn:** what Markdown emits, how links resolve, and which limits
> keep extraction local and bounded.
>
> **Prerequisites:** [Language architecture](language-architecture.md) and
> [Graph model](../concepts/graph-model.md).

## Ownership and data flow

```text
bounded source bytes
        |
        v
compass-languages::Engine
        |
        +--> pinned Markdown block grammar
        +--> pinned Markdown inline grammar
        +--> bounded frontmatter decoder
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

The parser grammars are statically linked through the pinned `tree-sitter-md`
crate. Markdown extraction does not download grammars, invoke Python, call a
model, or follow a URL.

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

## Cache and trust boundaries

Markdown file hashes include the raw frontmatter bytes. A metadata-only edit
therefore invalidates structural and semantic cache entries just like a body
edit. The legacy `body_content` helper remains available for callers that
explicitly need the old body-only view; it is not used as the graph cache key.

All parser work is bounded by the caller's source-read limits plus explicit
frontmatter, block, link, metadata, and diagnostic caps. Inputs are never
executed, fetched, or interpreted as configuration. Unknown graph attributes
remain extensible, and consumers should preserve them.

## Other document formats

File discovery recognizes several document extensions, but recognition is not
the same as structural extraction. HTML, DOCX, PPTX, RTF, and spreadsheet
normalizers remain separate integration surfaces until they have their own
bounded parser, exact locator policy, and qualification fixtures. See the
[document format reference](../reference/document-formats.md) for the current
matrix. Markdown links may point at those formats, but Compass does not fetch
or execute a linked resource during Markdown extraction.

## Related pages

- [Document format reference](../reference/document-formats.md)
- [Language architecture](language-architecture.md)
- [Extraction pipeline](../implementation/extraction-pipeline.md)
- [Graph model](../concepts/graph-model.md)
