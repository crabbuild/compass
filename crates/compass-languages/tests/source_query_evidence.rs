use compass_languages::Engine;
use serde_json::Value;
use std::error::Error;
use std::path::Path;

const EXTENSION: &str = "_compass_source_query_evidence";
fn extract(path: &str, source: &str) -> Result<Value, Box<dyn Error>> {
    let graph =
        Engine::default().extract_source_graph_only(Path::new(path), path, source.as_bytes())?;
    graph
        .extensions
        .get(EXTENSION)
        .cloned()
        .ok_or_else(|| "missing source-query projection".into())
}

#[test]
fn native_extraction_keeps_owned_body_vocabulary_and_exact_sites() -> Result<(), Box<dyn Error>> {
    for (path, source, name) in [
        (
            "src/sample.py",
            "# outside_marker\ndef opaque(payload):\n    annotations = payload\n    # retained marker\n    def inner():\n        return secret\n    return annotations\n",
            "opaque",
        ),
        (
            "src/sample.rs",
            "// outside_marker\nfn opaque(payload: &str) { let annotations = payload; // retained marker\nfn inner() { let secret = 0; } }",
            "opaque",
        ),
        (
            "sample.go",
            "package p\n// outside_marker\nfunc Opaque(payload string) { annotations := payload; // retained marker\ninner := func() { secret() }; _ = annotations; _ = inner }",
            "Opaque",
        ),
        (
            "src/sample.ts",
            "// outside_marker\nexport function opaque(payload: unknown) { const annotations = payload; // retained marker\nfunction inner() { const secret = 0; } return annotations; }",
            "opaque",
        ),
    ] {
        let value = extract(path, source)?;
        assert_eq!(value["schema"], "compass.source-query.extraction/1");
        assert_eq!(value["sourceFile"], path);
        assert_eq!(value["byteSize"], source.len());
        assert_eq!(value["syntaxRecovery"], false);
        let p = &value["projection"];
        assert_eq!(p["truncated"], false, "{path}");
        let scopes = p["scopes"].as_array().ok_or("missing scopes")?;
        let owner = scopes
            .iter()
            .find(|s| s["name"] == name)
            .ok_or("missing owner")?;
        let names = owner["names"].as_object().ok_or("missing names")?;
        assert!(
            names.contains_key("payload") && names.contains_key("annotations"),
            "{path}"
        );
        assert!(
            !names.contains_key("secret") && !names.contains_key(name),
            "{path}"
        );
        assert!(scopes.iter().any(|s| s["names"].get("secret").is_some()));
        let comments = owner["comments"].as_array().ok_or("missing comments")?;
        assert!(comments.iter().any(|s| {
            s["text"]
                .as_str()
                .is_some_and(|t| t.contains("retained marker"))
        }));
        assert!(!comments.iter().any(|s| {
            s["text"]
                .as_str()
                .is_some_and(|t| t.contains("outside_marker"))
        }));
        for scope in scopes {
            let start = scope["start"].as_u64().ok_or("scope start")?;
            let end = scope["end"].as_u64().ok_or("scope end")?;
            let names = scope["names"].as_object().ok_or("scope names")?;
            let sites = names
                .values()
                .flat_map(|v| v.as_array().into_iter().flatten())
                .chain(scope["comments"].as_array().into_iter().flatten());
            for site in sites {
                let a = site["start"].as_u64().ok_or("site start")?;
                let b = site["end"].as_u64().ok_or("site end")?;
                assert!(a >= start && b <= end);
                assert_eq!(
                    source.get(usize::try_from(a)?..usize::try_from(b)?),
                    site["text"].as_str()
                );
            }
        }
        // The payload survives the real extraction cache's serde representation.
        let decoded: Value = serde_json::from_slice(&serde_json::to_vec(&value)?)?;
        assert_eq!(value, decoded);
    }
    Ok(())
}

#[test]
fn class_boundaries_strings_and_unicode_are_preserved() -> Result<(), Box<dyn Error>> {
    let source = "function opaque(payload: unknown) { const café = 'secret_literal'; class Nested { x = classOnly; meth() { methodOnly(); } } // 雪 retained\nreturn café; }";
    let value = extract("src/sample.ts", source)?;
    let scopes = value["projection"]["scopes"]
        .as_array()
        .ok_or("missing scopes")?;
    let owner = scopes
        .iter()
        .find(|s| s["name"] == "opaque")
        .ok_or("missing owner")?;
    let names = owner["names"].as_object().ok_or("missing names")?;
    assert!(names.contains_key("café"));
    for term in ["secret_literal", "classOnly", "methodOnly", "Nested"] {
        assert!(!names.contains_key(term), "{term}");
    }
    assert!(
        scopes
            .iter()
            .any(|s| s["name"] == "meth" && s["names"].get("methodOnly").is_some())
    );
    Ok(())
}
