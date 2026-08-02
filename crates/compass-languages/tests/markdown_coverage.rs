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
    assert_eq!(references.len(), 9, "references={references:#?}");
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
        6
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
