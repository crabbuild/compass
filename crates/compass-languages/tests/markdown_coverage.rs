use std::error::Error;
use std::fs;

use compass_languages::Engine;

#[test]
fn markdown_extracts_heading_hierarchy_and_only_local_document_links() -> Result<(), Box<dyn Error>>
{
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("fixture.md");
    fs::write(
        &path,
        r#"# Root
[relative](docs/./guide)
[qualified](notes/page.mdx?mode=read#part)
[angle](<../shared/readme.qmd> "Shared")
[text](plain.txt)
[rst](reference.rst)
[long](chapter.markdown)
[[Wiki Page#Section|Alias]]
[definition]: refs/definition

![inline image](images/ignored.md)
![[Embedded Wiki]]
[duplicate](docs/guide.md)
[self](fixture.md)
[anchor](#root)
[query](?only=query)
[web](https://example.com/a.md)
[protocol](//example.com/a.md)
[mail](mailto:a@example.com)
[telephone](tel:123)
[data](data:text/plain,hello)
[asset](image.png)

```markdown
# Hidden
[hidden](hidden.md)
```

## Child
#### Deep
## Child
### Nested
# Reset
"#,
    )?;

    let extraction = Engine::default().extract(&path)?;
    assert!(extraction.error.is_none());
    let labels = extraction
        .nodes
        .iter()
        .map(|node| node.label())
        .collect::<Vec<_>>();
    for expected in ["fixture.md", "Root", "Child", "Deep", "Nested", "Reset"] {
        assert!(labels.contains(&expected), "missing {expected}: {labels:?}");
    }
    assert_eq!(labels.iter().filter(|label| **label == "Child").count(), 2);
    assert!(!labels.contains(&"Hidden"));
    let child_scopes = extraction
        .nodes
        .iter()
        .filter(|node| node.label() == "Child")
        .map(|node| node.string("qualified_name"))
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        child_scopes,
        ["Root::Child", "Root::Child#2"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    );

    let references = extraction
        .edges
        .iter()
        .filter(|edge| {
            edge.attributes
                .get("relation")
                .and_then(serde_json::Value::as_str)
                == Some("references")
        })
        .collect::<Vec<_>>();
    assert_eq!(references.len(), 10, "references={references:#?}");
    assert!(references.iter().all(|edge| {
        edge.attributes
            .get("confidence")
            .and_then(serde_json::Value::as_str)
            == Some("EXTRACTED")
    }));
    assert_eq!(
        extraction
            .edges
            .iter()
            .filter(|edge| {
                edge.attributes
                    .get("relation")
                    .and_then(serde_json::Value::as_str)
                    == Some("contains")
            })
            .count(),
        9
    );
    assert!(
        references
            .iter()
            .all(|edge| edge.source != extraction.nodes[0].id)
    );
    assert_eq!(extraction.extensions["input_tokens"], 0);
    assert_eq!(extraction.extensions["output_tokens"], 0);
    Ok(())
}

#[test]
fn markdown_missing_file_is_a_structured_io_error() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let error = match Engine::default().extract(&directory.path().join("absent.md")) {
        Ok(_) => return Err("missing Markdown unexpectedly succeeded".into()),
        Err(error) => error,
    };
    assert!(error.to_string().contains("absent.md"));
    Ok(())
}

#[test]
fn markdown_file_and_same_named_heading_have_distinct_identities() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("AGENTS.md");
    fs::write(&path, "# AGENTS.md\n")?;

    let extraction = Engine::default().extract(&path)?;
    assert!(extraction.error.is_none());
    assert_eq!(extraction.nodes.len(), 2);
    assert_ne!(extraction.nodes[0].id, extraction.nodes[1].id);
    assert_eq!(
        extraction.nodes[1].string("qualified_name"),
        "AGENTS.md::heading"
    );
    assert!(extraction.edges.iter().any(|edge| {
        edge.source == extraction.nodes[0].id
            && edge.target == extraction.nodes[1].id
            && edge
                .attributes
                .get("relation")
                .and_then(serde_json::Value::as_str)
                == Some("contains")
    }));
    Ok(())
}

#[test]
fn markdown_source_path_supports_frontmatter_blocks_and_section_links() -> Result<(), Box<dyn Error>>
{
    let path = std::path::Path::new("docs/guide.md");
    let source = br#"---
title: Guide
tags: [rust, graph]
draft: false
---
# Intro {#start}

Setext heading
---------------

- [x] checked item
  - nested item

| Name | Value |
| :--- | ---: |
| alpha | `one` |

```rust
let hidden = "[not a link](ignored.md)";
```

[guide][reference]
[reference]: ./reference.md
[jump](#start)
"#;

    let extraction = Engine::default().extract_source(path, source)?;
    let root = extraction
        .nodes
        .first()
        .ok_or("missing Markdown document root")?;
    assert_eq!(root.string("document_format"), "markdown");
    assert_eq!(root.attributes["document_metadata"]["title"], "Guide");
    assert_eq!(
        root.attributes["document_metadata"]["tags"],
        serde_json::json!(["rust", "graph"])
    );

    let kinds = extraction
        .nodes
        .iter()
        .filter_map(|node| node.attributes.get("document_kind"))
        .filter_map(serde_json::Value::as_str)
        .collect::<Vec<_>>();
    for expected in [
        "heading",
        "list",
        "list_item",
        "pipe_table",
        "pipe_table_header",
        "pipe_table_row",
        "pipe_table_cell",
        "fenced_code_block",
    ] {
        assert!(kinds.contains(&expected), "missing {expected}: {kinds:?}");
    }
    assert!(extraction.nodes.iter().any(|node| {
        node.attributes.get("heading_style") == Some(&serde_json::json!("setext"))
            && node.attributes.get("heading_level") == Some(&serde_json::json!(2))
    }));
    assert!(!extraction.edges.iter().any(|edge| {
        edge.attributes.get("link_kind") == Some(&serde_json::json!("inline"))
            && edge.target.contains("ignored")
    }));
    let reference_edges = extraction
        .edges
        .iter()
        .filter(|edge| edge.attributes.get("link_kind").is_some())
        .collect::<Vec<_>>();
    assert_eq!(reference_edges.len(), 3);
    assert!(reference_edges.iter().any(|edge| {
        edge.attributes.get("link_kind") == Some(&serde_json::json!("reference_definition"))
            && edge.attributes.get("relation") == Some(&serde_json::json!("references"))
    }));
    assert!(reference_edges.iter().any(|edge| {
        edge.attributes.get("fragment") == Some(&serde_json::json!("start"))
            && edge.target != root.id
    }));
    assert!(extraction.nodes.iter().all(|node| {
        node.attributes
            .get("start_byte")
            .and_then(serde_json::Value::as_u64)
            .is_some_and(|start| {
                node.attributes
                    .get("end_byte")
                    .and_then(serde_json::Value::as_u64)
                    .is_some_and(|end| start <= end && (end as usize) <= source.len())
            })
    }));

    let combined = Engine::default().extract_source_combined(
        std::path::Path::new("/workspace/docs/guide.md"),
        "docs/guide.md",
        source,
    )?;
    assert!(combined.program.is_none());
    assert_eq!(
        combined.graph.nodes[0].string("source_file"),
        "docs/guide.md"
    );
    Ok(())
}

#[test]
fn markdown_duplicate_fragments_are_explicitly_unresolved() -> Result<(), Box<dyn Error>> {
    let extraction = Engine::default().extract_source(
        std::path::Path::new("guide.md"),
        b"# Same\n\n## Same\n\n## Other\n\n[jump](#same)\n",
    )?;
    assert!(
        !extraction
            .edges
            .iter()
            .any(|edge| edge.attributes.get("link_kind") == Some(&serde_json::json!("inline")))
    );
    let unresolved = extraction
        .extensions
        .get("markdown_unresolved_links")
        .and_then(serde_json::Value::as_array)
        .ok_or("missing unresolved Markdown link evidence")?;
    assert!(unresolved.iter().any(|value| {
        value.get("reason").and_then(serde_json::Value::as_str) == Some("ambiguous_fragment")
    }));
    Ok(())
}

#[test]
fn markdown_links_are_owned_by_the_smallest_structural_block() -> Result<(), Box<dyn Error>> {
    let extraction = Engine::default().extract_source(
        std::path::Path::new("guide.md"),
        b"- [item](item.md)\n  - [nested](nested.md)\n\n> [quote](quote.md)\n",
    )?;

    let link_edges = extraction
        .edges
        .iter()
        .filter(|edge| edge.attributes.get("link_kind").is_some())
        .collect::<Vec<_>>();
    assert_eq!(link_edges.len(), 3);
    for edge in link_edges {
        let source = extraction
            .nodes
            .iter()
            .find(|node| node.id == edge.source)
            .ok_or("missing link owner")?;
        assert_eq!(
            source.attributes.get("document_kind"),
            Some(&serde_json::json!("paragraph"))
        );
    }
    let root_id = &extraction.nodes[0].id;
    assert!(extraction.edges.iter().any(|edge| {
        edge.source.as_str() == root_id.as_str()
            && edge.attributes.get("relation") == Some(&serde_json::json!("contains"))
    }));
    assert!(extraction.nodes.iter().all(|node| {
        !matches!(
            node.attributes
                .get("document_kind")
                .and_then(serde_json::Value::as_str),
            Some("block_continuation")
                | Some("block_quote_marker")
                | Some("link_destination")
                | Some("list_marker_minus")
        )
    }));
    Ok(())
}

#[test]
fn markdown_frontmatter_is_bounded_and_diagnosed_without_swallowing_body()
-> Result<(), Box<dyn Error>> {
    let indented = Engine::default().extract_source(
        std::path::Path::new("guide.md"),
        b" ---\ntitle: not metadata\n---\n# Body\n",
    )?;
    assert!(
        !indented.nodes[0]
            .attributes
            .contains_key("document_metadata")
    );
    assert!(indented.nodes.iter().any(|node| node.label() == "Body"));

    let malformed = Engine::default().extract_source(
        std::path::Path::new("guide.md"),
        b"---\ntitle: [unterminated\n---\n# Body\n",
    )?;
    let diagnostics = malformed
        .extensions
        .get("markdown_diagnostics")
        .and_then(serde_json::Value::as_array)
        .ok_or("missing frontmatter diagnostic")?;
    assert!(diagnostics.iter().any(|value| {
        value
            .as_str()
            .is_some_and(|message| message.contains("frontmatter"))
    }));
    assert!(malformed.nodes.iter().any(|node| node.label() == "Body"));

    let unclosed = Engine::default().extract_source(
        std::path::Path::new("guide.md"),
        b"---\ntitle: still body\n# Body\n",
    )?;
    let unclosed_diagnostics = unclosed
        .extensions
        .get("markdown_diagnostics")
        .and_then(serde_json::Value::as_array)
        .ok_or("missing unclosed frontmatter diagnostic")?;
    assert!(unclosed_diagnostics.iter().any(|value| {
        value
            .as_str()
            .is_some_and(|message| message.contains("closing delimiter"))
    }));
    assert!(unclosed.nodes.iter().any(|node| node.label() == "Body"));

    let unsafe_yaml = Engine::default().extract_source(
        std::path::Path::new("guide.md"),
        b"---\ntitle: \"R&D\"\nbase: &shared value\ncopy: *shared\n---\n# Body\n",
    )?;
    let unsafe_diagnostics = unsafe_yaml
        .extensions
        .get("markdown_diagnostics")
        .and_then(serde_json::Value::as_array)
        .ok_or("missing alias diagnostic")?;
    assert!(unsafe_diagnostics.iter().any(|value| {
        value
            .as_str()
            .is_some_and(|message| message.contains("aliases and tags"))
    }));
    assert!(
        !unsafe_yaml.nodes[0]
            .attributes
            .contains_key("document_metadata")
    );
    assert!(unsafe_yaml.nodes.iter().any(|node| node.label() == "Body"));
    Ok(())
}

#[test]
fn markdown_footnotes_and_mdx_quarto_constructs_remain_explicit() -> Result<(), Box<dyn Error>> {
    let mdx = Engine::default().extract_source(
        std::path::Path::new("guide.mdx"),
        b"import Callout from './Callout.jsx'\n\n# Guide\n\nText[^note]\n\n<Callout>content</Callout>\n{value}\n\n[^note]: A bounded footnote.\n",
    )?;
    assert!(mdx.nodes.iter().any(|node| {
        node.attributes.get("document_kind") == Some(&serde_json::json!("other"))
            && node.attributes.get("other_kind") == Some(&serde_json::json!("mdx_construct"))
    }));
    assert!(mdx.nodes.iter().any(|node| {
        node.attributes.get("document_kind") == Some(&serde_json::json!("footnote_definition"))
            && node.attributes.get("footnote_label") == Some(&serde_json::json!("note"))
    }));
    assert!(mdx.edges.iter().any(|edge| {
        edge.attributes.get("link_kind") == Some(&serde_json::json!("footnote"))
            && edge.attributes.get("relation") == Some(&serde_json::json!("references"))
    }));

    let qmd = Engine::default().extract_source(
        std::path::Path::new("report.qmd"),
        b"::: {.callout-note}\nQuarto content\n:::\n\n{{< include shared.qmd >}}\n",
    )?;
    assert!(qmd.nodes.iter().any(|node| {
        node.attributes.get("document_kind") == Some(&serde_json::json!("other"))
            && node.attributes.get("other_kind") == Some(&serde_json::json!("quarto_directive"))
    }));
    assert!(
        qmd.extensions["markdown_other_count"]
            .as_u64()
            .is_some_and(|count| count >= 2)
    );
    Ok(())
}
