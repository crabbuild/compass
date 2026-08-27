use std::error::Error;
use std::fs;

use compass_languages::Engine;

const MAX_TEST_FRONTMATTER_DEPTH: usize = 13;

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
site:
  navigation:
    label: Graph guide
routes:
  "docs/url": /guide
authors:
  - name: Ada
    roles: [editor, reviewer]
reviewers: [{name: Grace, team: Docs}]
aliases:
  - Compass guide
  - Graph handbook
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
    let metadata = root
        .attributes
        .get("document_metadata")
        .unwrap_or_else(|| panic!("extensions={:#?}", extraction.extensions));
    assert_eq!(metadata["title"], "Guide");
    assert_eq!(
        root.attributes["document_metadata"]["tags"],
        serde_json::json!(["rust", "graph"])
    );
    assert_eq!(
        root.attributes["document_metadata"]["site"]["navigation"]["label"],
        "Graph guide"
    );
    assert_eq!(
        root.attributes["document_metadata"]["authors"][0]["roles"],
        serde_json::json!(["editor", "reviewer"])
    );
    assert_eq!(root.label(), "Guide");
    assert_eq!(root.string("qualified_name"), "docs/guide.md");

    let config_nodes = extraction
        .nodes
        .iter()
        .filter(|node| node.string("symbol_kind") == "config_key")
        .collect::<Vec<_>>();
    for expected in [
        "frontmatter/title",
        "frontmatter/tags",
        "frontmatter/site",
        "frontmatter/site/navigation",
        "frontmatter/site/navigation/label",
        "frontmatter/routes",
        "frontmatter/routes/docs~1url",
        "frontmatter/authors",
        "frontmatter/authors/0",
        "frontmatter/authors/0/name",
        "frontmatter/authors/0/roles",
        "frontmatter/reviewers",
        "frontmatter/reviewers/0",
        "frontmatter/reviewers/0/name",
        "frontmatter/reviewers/0/team",
        "frontmatter/aliases",
    ] {
        assert!(
            config_nodes
                .iter()
                .any(|node| node.string("qualified_name") == expected),
            "missing {expected}: {config_nodes:#?}"
        );
    }
    assert!(config_nodes.iter().all(|node| {
        node.string("format") == "yaml_frontmatter"
            && node.string("file_type") == "code"
            && node.string("_origin") == "config"
            && node
                .attributes
                .get("start_byte")
                .and_then(serde_json::Value::as_u64)
                .zip(
                    node.attributes
                        .get("end_byte")
                        .and_then(serde_json::Value::as_u64),
                )
                .is_some_and(|(start, end)| start < end)
    }));
    let title = config_nodes
        .iter()
        .find(|node| node.string("qualified_name") == "frontmatter/title")
        .ok_or("missing title metadata node")?;
    assert_eq!(title.label(), "title: Guide");
    let author = config_nodes
        .iter()
        .find(|node| node.string("qualified_name") == "frontmatter/authors/0")
        .ok_or("missing author metadata node")?;
    let author_name = config_nodes
        .iter()
        .find(|node| node.string("qualified_name") == "frontmatter/authors/0/name")
        .ok_or("missing author name metadata node")?;
    assert!(extraction.edges.iter().any(|edge| {
        edge.source == author.id
            && edge.target == author_name.id
            && edge.string("relation") == "contains"
            && edge.string("_origin") == "config"
    }));

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
    let table = extraction
        .nodes
        .iter()
        .find(|node| node.attributes.get("document_kind") == Some(&serde_json::json!("pipe_table")))
        .ok_or("missing table")?;
    assert_eq!(table.label(), "Intro::Setext heading — table: Name | Value");
    assert_eq!(
        table.attributes["table_headers"],
        serde_json::json!(["Name", "Value"])
    );
    assert_eq!(
        table.attributes["table_alignments"],
        serde_json::json!(["left", "right"])
    );
    assert_eq!(
        table.attributes["table_body_row_count"],
        serde_json::json!(1)
    );
    let row = extraction
        .nodes
        .iter()
        .find(|node| {
            node.attributes.get("document_kind") == Some(&serde_json::json!("pipe_table_row"))
        })
        .ok_or("missing table row")?;
    assert_eq!(row.label(), "Name=alpha · Value=`one`");
    assert_eq!(row.attributes["table_row_index"], serde_json::json!(0));
    assert_eq!(
        row.attributes["table_identity_cell_index"],
        serde_json::json!(0)
    );
    assert_eq!(
        row.attributes["table_cells"][0]["state"],
        serde_json::json!("present")
    );
    assert_eq!(
        row.attributes["table_cells"][0]["text"],
        serde_json::json!("alpha")
    );
    assert_eq!(
        row.attributes["table_cells"][1]["text"],
        serde_json::json!("`one`")
    );
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
    assert!(extraction.nodes.iter().any(|node| {
        node.attributes.get("document_kind") == Some(&serde_json::json!("paragraph"))
            && node.string("qualified_name").contains("::paragraph#")
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
fn markdown_tables_publish_semantic_rows_and_retain_cell_links() -> Result<(), Box<dyn Error>> {
    let source = br#"# Ownership

| Area | Owner | Status |
| :--- | :---: | ---: |
| [Graph](../graph.md) | `compass-model` | active |
| Empty | | values |
| Missing |

## After
"#;
    let extraction =
        Engine::default().extract_source(std::path::Path::new("docs/index.md"), source)?;
    let tables = extraction
        .nodes
        .iter()
        .filter(|node| {
            node.attributes.get("document_kind") == Some(&serde_json::json!("pipe_table"))
        })
        .collect::<Vec<_>>();
    let rows = extraction
        .nodes
        .iter()
        .filter(|node| {
            node.attributes.get("document_kind") == Some(&serde_json::json!("pipe_table_row"))
        })
        .collect::<Vec<_>>();
    let headers = extraction
        .nodes
        .iter()
        .filter(|node| {
            node.attributes.get("document_kind") == Some(&serde_json::json!("pipe_table_header"))
        })
        .collect::<Vec<_>>();
    let cells = extraction
        .nodes
        .iter()
        .filter(|node| {
            node.attributes.get("document_kind") == Some(&serde_json::json!("pipe_table_cell"))
        })
        .collect::<Vec<_>>();
    assert_eq!(tables.len(), 1);
    assert_eq!(headers.len(), 1);
    assert_eq!(rows.len(), 3);
    assert_eq!(cells.len(), 10);
    assert_eq!(
        tables[0].attributes["table_body_row_count"],
        serde_json::json!(3)
    );
    assert_eq!(
        tables[0].attributes["table_headers"],
        serde_json::json!(["Area", "Owner", "Status"])
    );
    assert_eq!(
        tables[0].attributes["table_alignments"],
        serde_json::json!(["left", "center", "right"])
    );
    assert_eq!(
        rows[0].label(),
        "Area=[Graph](../graph.md) · Owner=`compass-model` · Status=active"
    );
    assert_eq!(rows[1].label(), "Area=Empty · Status=values");
    assert_eq!(
        rows[1].attributes["table_cells"][1]["state"],
        serde_json::json!("empty")
    );
    assert_eq!(rows[2].label(), "Area=Missing");
    assert_eq!(
        rows[2].attributes["table_cells"][1]["state"],
        serde_json::json!("missing")
    );
    assert_eq!(
        rows[2].attributes["table_cells"][2]["state"],
        serde_json::json!("missing")
    );
    let table_id = &tables[0].id;
    let linked_row = rows[0];
    let link_cell = cells
        .iter()
        .find(|node| node.label().starts_with("Area: [Graph]"))
        .ok_or("missing linked body cell")?;
    assert!(extraction.edges.iter().any(|edge| {
        edge.source == link_cell.id
            && edge.attributes.get("relation") == Some(&serde_json::json!("references"))
            && edge.attributes.get("link_kind") == Some(&serde_json::json!("inline"))
    }));
    let references = link_cell
        .attributes
        .get("document_references")
        .and_then(serde_json::Value::as_array)
        .ok_or("missing cell reference evidence")?;
    assert!(references.iter().any(|reference| {
        reference.get("kind") == Some(&serde_json::json!("inline"))
            && reference.get("resolution") == Some(&serde_json::json!("exact"))
            && reference["site"]["startByte"].as_u64().is_some()
    }));
    let code_cell = cells
        .iter()
        .find(|node| node.label() == "Owner: `compass-model`")
        .ok_or("missing inline-code body cell")?;
    let code_references = code_cell
        .attributes
        .get("document_references")
        .and_then(serde_json::Value::as_array)
        .ok_or("missing inline-code cell evidence")?;
    assert!(code_references.iter().any(|reference| {
        reference.get("kind") == Some(&serde_json::json!("inline_code"))
            && reference.get("spelling") == Some(&serde_json::json!("compass-model"))
            && reference.get("resolution") == Some(&serde_json::json!("unresolved"))
    }));
    assert_eq!(
        references
            .iter()
            .filter(|reference| reference.get("kind") == Some(&serde_json::json!("inline")))
            .count(),
        1
    );
    assert!(extraction.edges.iter().any(|edge| {
        edge.source == *table_id
            && edge.target == linked_row.id
            && edge.attributes.get("relation") == Some(&serde_json::json!("contains"))
    }));
    Ok(())
}

#[test]
fn markdown_table_limits_are_truthful_and_later_blocks_survive() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("limits.md");
    let mut source = String::from("# Limits\n| Key | Value |\n| --- | --- |\n");
    for index in 0..10_005 {
        source.push_str(&format!("| item-{index} | retained |\n"));
    }
    source.push_str("\n# After\nThe later section remains extractable.\n");
    std::fs::write(&path, source)?;

    let extraction = Engine::default().extract(&path)?;
    let table = extraction
        .nodes
        .iter()
        .find(|node| node.attributes.get("document_kind") == Some(&serde_json::json!("pipe_table")))
        .ok_or("missing limited table")?;
    let retained = table.attributes["table_body_row_count"]
        .as_u64()
        .ok_or("missing retained row count")?;
    let omitted = table.attributes["table_omitted_row_count"]
        .as_u64()
        .ok_or("missing omitted row count")?;
    assert!(retained > 0 && retained < 10_005);
    assert_eq!(retained + omitted, 10_005);
    assert_eq!(table.attributes["table_truncated"], serde_json::json!(true));
    assert_eq!(
        extraction.extensions[compass_languages::EXTRACTION_QUALITY_EXTENSION],
        serde_json::json!(compass_languages::EXTRACTION_QUALITY_PARTIAL)
    );
    assert!(extraction.nodes.iter().any(|node| {
        node.attributes.get("document_kind") == Some(&serde_json::json!("heading"))
            && node.label() == "After"
    }));
    Ok(())
}

#[test]
fn markdown_table_anchors_are_byte_exact_across_crlf_and_unicode() -> Result<(), Box<dyn Error>> {
    let source = "# Зона\r\n\r\n| Name | Value |\r\n| --- | --- |\r\n| café | `one\\|two` |\r\n";
    let extraction = Engine::default()
        .extract_source(std::path::Path::new("docs/guide.md"), source.as_bytes())?;
    let table = extraction
        .nodes
        .iter()
        .find(|node| node.attributes.get("document_kind") == Some(&serde_json::json!("pipe_table")))
        .ok_or("missing table")?;
    let column_anchor = &table.attributes["table_columns"][0]["source"];
    let name_start = source.find("Name").ok_or("missing header text")?;
    assert_eq!(column_anchor["startByte"], serde_json::json!(name_start));
    // Tree-sitter's cell span includes the trailing cell whitespace while
    // preserving the visible text's exact byte start.
    assert_eq!(column_anchor["endByte"], serde_json::json!(name_start + 5));
    assert_eq!(column_anchor["startLine"], serde_json::json!(3));
    assert_eq!(column_anchor["endLine"], serde_json::json!(3));

    let row = extraction
        .nodes
        .iter()
        .find(|node| {
            node.attributes.get("document_kind") == Some(&serde_json::json!("pipe_table_row"))
        })
        .ok_or("missing row")?;
    assert_eq!(
        row.attributes["table_cells"][0]["text"],
        serde_json::json!("café")
    );
    assert_eq!(
        row.attributes["table_cells"][0]["source"]["startLine"],
        serde_json::json!(5)
    );
    assert_eq!(
        row.attributes["table_cells"][1]["text"],
        serde_json::json!("`one\\|two`")
    );
    Ok(())
}

#[test]
fn markdown_duplicate_heading_slugs_follow_source_order_and_explicit_ids_remain_ambiguous()
-> Result<(), Box<dyn Error>> {
    let extraction = Engine::default().extract_source(
        std::path::Path::new("guide.md"),
        b"# Same\n\n## Same\n\n## Other\n\n[first](#same) [second](#same-1)\n",
    )?;
    let links = extraction
        .edges
        .iter()
        .filter(|edge| edge.attributes.get("link_kind") == Some(&serde_json::json!("inline")))
        .collect::<Vec<_>>();
    assert_eq!(links.len(), 2);
    assert_ne!(links[0].target, links[1].target);
    let slugs = extraction
        .nodes
        .iter()
        .filter_map(|node| node.attributes.get("anchor_slug"))
        .filter_map(serde_json::Value::as_str)
        .collect::<Vec<_>>();
    assert!(slugs.contains(&"same"));
    assert!(slugs.contains(&"same-1"));

    let encoded = Engine::default().extract_source(
        std::path::Path::new("encoded.md"),
        b"# Encoded {#agent:rules}\n\n[jump](#agent%3Arules)\n",
    )?;
    let encoded_heading = encoded
        .nodes
        .iter()
        .find(|node| node.string("explicit_id") == "agent:rules")
        .ok_or("missing encoded-fragment heading")?;
    assert!(encoded.edges.iter().any(|edge| {
        edge.attributes.get("fragment") == Some(&serde_json::json!("agent%3Arules"))
            && edge.target == encoded_heading.id
    }));

    let explicit = Engine::default().extract_source(
        std::path::Path::new("explicit.md"),
        b"# One {#shared}\n\n# Two {#shared}\n\n[jump](#shared)\n",
    )?;
    let unresolved = explicit
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

    let duplicate = Engine::default().extract_source(
        std::path::Path::new("guide.md"),
        b"---\ntitle: first\ntitle: second\n---\n# Body\n",
    )?;
    assert!(
        !duplicate.nodes[0]
            .attributes
            .contains_key("document_metadata")
    );
    assert!(
        duplicate
            .extensions
            .get("markdown_diagnostics")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|diagnostics| diagnostics.iter().any(|value| value
                .as_str()
                .is_some_and(|message| message.contains("frontmatter"))))
    );
    assert!(duplicate.nodes.iter().any(|node| node.label() == "Body"));

    let invalid_utf8 = Engine::default().extract_source(
        std::path::Path::new("guide.md"),
        b"---\ntitle: \xff\n---\n# Body\n",
    )?;
    assert!(
        invalid_utf8
            .extensions
            .get("markdown_diagnostics")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|diagnostics| diagnostics.iter().any(|value| value
                .as_str()
                .is_some_and(|message| message.contains("valid UTF-8"))))
    );
    assert!(invalid_utf8.nodes.iter().any(|node| node.label() == "Body"));

    let mut deeply_nested = String::from("---\n");
    for depth in 0..=MAX_TEST_FRONTMATTER_DEPTH {
        deeply_nested.push_str(&format!("{}level{depth}:\n", "  ".repeat(depth)));
    }
    deeply_nested.push_str(&format!(
        "{}value: final\n---\n# Body\n",
        "  ".repeat(MAX_TEST_FRONTMATTER_DEPTH + 1)
    ));
    let deeply_nested = Engine::default()
        .extract_source(std::path::Path::new("guide.md"), deeply_nested.as_bytes())?;
    assert!(
        !deeply_nested.nodes[0]
            .attributes
            .contains_key("document_metadata")
    );
    assert!(
        deeply_nested
            .extensions
            .get("markdown_diagnostics")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|diagnostics| diagnostics.iter().any(|value| value
                .as_str()
                .is_some_and(|message| message.contains("nesting-depth"))))
    );
    assert!(
        deeply_nested
            .nodes
            .iter()
            .any(|node| node.label() == "Body")
    );
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
