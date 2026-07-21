use std::error::Error;
use std::fs;

use trail_languages::Engine;

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
    assert_eq!(references.len(), 8, "references={references:#?}");
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
