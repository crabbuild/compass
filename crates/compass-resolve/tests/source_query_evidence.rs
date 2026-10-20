use compass_graph::{BuildEvidence, canonical_graph_json, normalize_v1};
use compass_languages::{Engine, source_query::EXTRACTION_EXTENSION};
use compass_resolve::resolve_with_root;
use std::collections::HashMap;
use std::error::Error;

#[test]
fn lexical_body_metadata_does_not_change_published_code_records() -> Result<(), Box<dyn Error>> {
    for (file, source) in [
        (
            "src/sample.py",
            "def helper(payload):\n    return payload\ndef opaque(payload):\n    # handle annotations\n    return helper(payload)\n",
        ),
        (
            "src/sample.rs",
            "fn helper(payload: &str) -> &str { payload }\nfn opaque(payload: &str) -> &str { // handle annotations\nhelper(payload) }\n",
        ),
        (
            "sample.go",
            "package sample\nfunc helper(payload string) string { return payload }\nfunc Opaque(payload string) string { // handle annotations\nreturn helper(payload) }\n",
        ),
        (
            "src/sample.ts",
            "function helper(payload: unknown) { return payload; }\nexport function opaque(payload: unknown) { // handle annotations\nreturn helper(payload); }\n",
        ),
    ] {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join(file);
        std::fs::create_dir_all(path.parent().ok_or("missing parent")?)?;
        std::fs::write(&path, source)?;
        let extraction =
            Engine::default().extract_source_graph_only(&path, file, source.as_bytes())?;
        assert!(extraction.error.is_none(), "{file}: {:?}", extraction.error);
        let mut without = extraction.clone();
        assert!(
            without.extensions.remove(EXTRACTION_EXTENSION).is_some(),
            "{file}"
        );
        let sources = HashMap::from([(path.to_string_lossy().into_owned(), source.to_owned())]);
        let mut canonical = Vec::new();
        for input in [extraction, without] {
            let resolved = resolve_with_root(&[input], &sources, directory.path());
            let evidence = BuildEvidence::from_extraction(
                directory.path(),
                &resolved,
                "sha256:source-query-invariance",
            )?;
            let graph = normalize_v1(resolved, evidence)?;
            compass_model::validate_code_graph(&graph)?;
            assert!(!graph.nodes.is_empty() && !graph.links.is_empty(), "{file}");
            canonical.push(canonical_graph_json(&graph)?);
        }
        assert_eq!(
            canonical[0], canonical[1],
            "query-only evidence changed graph records: {file}"
        );
    }
    Ok(())
}
