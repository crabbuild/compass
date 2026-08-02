#![allow(clippy::expect_used)]

use std::collections::BTreeSet;
use std::error::Error;
use std::path::Path;

use compass_languages::{Engine, Registry};
use serde_json::Value;

#[test]
fn html_emits_semantic_structure_metadata_and_link_provenance() -> Result<(), Box<dyn Error>> {
    let source = br##"<!doctype html>
<html><head>
  <title>Guide &amp; API</title>
  <meta name="description" content="A &amp; guide">
  <base href="https://docs.example.test/guide/">
  <link rel="canonical" href="https://docs.example.test/guide/index.html">
  <style>.hidden { display: none }</style>
</head><body>
  <main id="intro"><h1>Guide</h1>
    <p>Read <a href="#intro" rel="self">this section</a> or <a href="https://example.test/api">the API</a>.</p>
    <nav><ul><li>Overview</li><li><a href="next.html">Next</a></li></ul></nav>
    <blockquote>Quoted</blockquote>
    <pre><code>const hidden = true;</code></pre>
    <table><tr><th>Name</th><th>Value</th></tr><tr><td>A</td><td>1</td></tr></table>
    <script>secret()</script><template>template secret</template><noscript>noscript secret</noscript>
  </main>
</body></html>"##;
    let path = Path::new("docs/index.html");
    let extraction = Engine::default().extract_source(path, source)?;
    let root = extraction
        .nodes
        .iter()
        .find(|node| {
            node.attributes.get("document_kind") == Some(&Value::String("document".to_owned()))
        })
        .expect("HTML root");
    assert_eq!(root.string("document_format"), "html");
    assert_eq!(root.string("html_title"), "Guide & API");
    assert!(root.string("html_visible_text").contains("Guide"));
    assert!(!root.string("html_visible_text").contains("secret"));
    assert_eq!(
        root.string("html_canonical"),
        "https://docs.example.test/guide/index.html"
    );
    assert_eq!(root.attributes["html_meta"]["description"], "A & guide");

    let kinds = extraction
        .nodes
        .iter()
        .filter_map(|node| node.attributes.get("document_kind").and_then(Value::as_str))
        .collect::<BTreeSet<_>>();
    for kind in [
        "heading",
        "paragraph",
        "landmark",
        "list",
        "list_item",
        "blockquote",
        "preformatted",
        "code",
        "table",
        "table_row",
        "table_cell",
        "link",
    ] {
        assert!(kinds.contains(kind), "missing HTML kind {kind}");
    }
    let paragraph_id = extraction
        .nodes
        .iter()
        .find(|node| {
            node.attributes.get("document_kind") == Some(&Value::String("paragraph".to_owned()))
        })
        .map(|node| node.id.clone())
        .expect("paragraph owner");
    assert!(
        extraction
            .edges
            .iter()
            .any(|edge| edge.attributes.get("link_kind")
                == Some(&Value::String("anchor".to_owned()))
                && edge.source == paragraph_id
                && edge.attributes.get("fragment") == Some(&Value::String("intro".to_owned())))
    );
    assert!(
        extraction
            .extensions
            .get("html_external_links")
            .and_then(Value::as_array)
            .is_some_and(|links| links
                .iter()
                .any(|link| link["target"] == "https://example.test/api"))
    );
    assert!(
        extraction
            .edges
            .iter()
            .any(|edge| edge.target == "docs_next_html")
    );
    assert!(
        extraction
            .edges
            .iter()
            .filter(|edge| edge.attributes.get("relation")
                == Some(&Value::String("contains".to_owned())))
            .all(|edge| edge.attributes.contains_key("start_byte")
                && edge.attributes.contains_key("end_byte"))
    );
    Ok(())
}

#[test]
fn html_extension_registry_is_structural_and_source_driven() -> Result<(), Box<dyn Error>> {
    let html = Registry::resolve(Path::new("page.htm")).expect("HTML registry case");
    assert_eq!(html.name, "html");
    assert_eq!(html.kind, compass_languages::ExtractorKind::Html);
    let combined = Engine::default().extract_source_combined(
        Path::new("page.htm"),
        "page.htm",
        b"<main><h2>Source driven</h2></main>",
    )?;
    assert!(combined.program.is_none());
    assert_eq!(combined.graph.nodes[0].string("document_format"), "html");
    Ok(())
}

#[test]
fn html_link_evidence_preserves_unsupported_and_remote_targets() -> Result<(), Box<dyn Error>> {
    let local = Engine::default().extract_source(
        Path::new("docs/index.html"),
        br#"<p><a href="asset.bin">Binary asset</a></p>"#,
    )?;
    assert!(
        local
            .extensions
            .get("html_unresolved_links")
            .and_then(Value::as_array)
            .is_some_and(|links| links.iter().any(|link| {
                link["reason"] == "unsupported_local_suffix" && link["target"] == "docs/asset.bin"
            }))
    );

    let remote = Engine::default().extract_source_combined(
        Path::new("index.html"),
        "https://example.test/start.html",
        br#"<p><a href="next">Next</a></p>"#,
    )?;
    assert!(
        remote
            .graph
            .extensions
            .get("html_external_links")
            .and_then(Value::as_array)
            .is_some_and(|links| links.iter().any(|link| {
                link["target"] == "https://example.test/next" && link["link_kind"] == "anchor"
            }))
    );
    Ok(())
}

#[test]
fn malformed_html_publishes_bounded_recovery_diagnostics() -> Result<(), Box<dyn Error>> {
    let extraction = Engine::default().extract_source(
        Path::new("broken.html"),
        b"<main><h1>Guide</h1><p>unclosed <a href=\"#missing\">link",
    )?;
    assert_eq!(
        extraction
            .extensions
            .get("_compass_extraction_quality")
            .and_then(Value::as_str),
        Some("partial")
    );
    assert!(
        extraction
            .extensions
            .get("html_diagnostics")
            .and_then(Value::as_array)
            .is_some_and(|diagnostics| !diagnostics.is_empty())
    );
    Ok(())
}
