use std::collections::BTreeSet;

use compass_model::GraphDocument;

use super::model::{ArchitectureOverlay, ArchitectureSourceScope};

#[derive(Clone, Debug)]
pub struct ScopeEvidence {
    pub scope: ArchitectureSourceScope,
    pub reason: &'static str,
}

#[must_use]
pub fn normalized_path(path: &str) -> String {
    path.replace('\\', "/")
        .trim()
        .trim_start_matches("./")
        .trim_matches('/')
        .to_ascii_lowercase()
}

#[must_use]
pub fn generated_paths(document: &GraphDocument) -> BTreeSet<String> {
    document
        .graph
        .get("files")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|record| {
            let record = record.as_object()?;
            record
                .get("generated")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
                .then(|| {
                    record
                        .get("path")
                        .and_then(serde_json::Value::as_str)
                        .map(normalized_path)
                })
                .flatten()
        })
        .collect()
}

#[must_use]
pub fn classify_source(
    source_file: Option<&str>,
    generated: &BTreeSet<String>,
    overlay: Option<&ArchitectureOverlay>,
) -> ScopeEvidence {
    let Some(source_file) = source_file.map(str::trim).filter(|path| !path.is_empty()) else {
        return ScopeEvidence {
            scope: ArchitectureSourceScope::Unknown,
            reason: "missing_source",
        };
    };
    let normalized = normalized_path(source_file);
    if let Some(rule) = overlay.and_then(|overlay| {
        overlay
            .source_rules
            .iter()
            .find(|rule| path_matches_prefix(&normalized, &normalized_path(&rule.path_prefix)))
    }) {
        return ScopeEvidence {
            scope: rule.scope,
            reason: "overlay",
        };
    }
    let segments = normalized.split('/').collect::<Vec<_>>();
    let filename = segments.last().copied().unwrap_or_default();
    if segments.iter().any(|segment| {
        matches!(
            *segment,
            "vendor"
                | "third_party"
                | "third-party"
                | "node_modules"
                | ".venv"
                | "venv"
                | "site-packages"
        )
    }) {
        return ScopeEvidence {
            scope: ArchitectureSourceScope::Vendor,
            reason: "vendor_path",
        };
    }
    if generated.contains(&normalized) {
        return ScopeEvidence {
            scope: ArchitectureSourceScope::Generated,
            reason: "graph_generated",
        };
    }
    let root_output = segments.first().is_some_and(|segment| {
        matches!(
            *segment,
            "generated" | "gen" | "dist" | "build" | "target" | ".next"
        )
    });
    let explicit_generated = segments
        .iter()
        .any(|segment| matches!(*segment, "generated" | "gen"));
    if root_output
        || explicit_generated
        || filename.contains(".generated.")
        || filename.contains("_generated.")
        || filename.ends_with(".min.js")
        || filename.ends_with(".min.css")
    {
        return ScopeEvidence {
            scope: ArchitectureSourceScope::Generated,
            reason: "generated_path",
        };
    }
    if segments
        .iter()
        .any(|segment| matches!(*segment, "test" | "tests" | "testing" | "__tests__"))
        || filename.starts_with("test_")
        || filename.contains("_test.")
        || filename.contains(".test.")
        || filename.contains(".spec.")
    {
        return ScopeEvidence {
            scope: ArchitectureSourceScope::Test,
            reason: "test_path",
        };
    }
    if segments
        .iter()
        .any(|segment| matches!(*segment, "doc" | "docs" | "documentation"))
        || matches!(
            filename,
            "readme" | "changelog" | "contributing" | "migration" | "security"
        )
        || filename.ends_with(".md")
        || filename.ends_with(".mdx")
        || filename.ends_with(".rst")
        || filename.ends_with(".adoc")
    {
        return ScopeEvidence {
            scope: ArchitectureSourceScope::Documentation,
            reason: "documentation_path",
        };
    }
    ScopeEvidence {
        scope: ArchitectureSourceScope::Production,
        reason: "source_path",
    }
}

#[must_use]
pub fn path_matches_prefix(path: &str, prefix: &str) -> bool {
    !prefix.is_empty()
        && (path == prefix
            || path
                .strip_prefix(prefix)
                .is_some_and(|remainder| remainder.starts_with('/')))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_scope_precedence_is_conservative() {
        let generated = BTreeSet::from(["assets/viewer/graph.js".to_owned()]);
        assert_eq!(
            classify_source(Some("vendor/pkg/tests/a.rs"), &generated, None).scope,
            ArchitectureSourceScope::Vendor
        );
        assert_eq!(
            classify_source(Some("assets/viewer/graph.js"), &generated, None).scope,
            ArchitectureSourceScope::Generated
        );
        assert_eq!(
            classify_source(Some("src/foo.test.ts"), &generated, None).scope,
            ArchitectureSourceScope::Test
        );
        assert_eq!(
            classify_source(Some("src/lib.rs"), &generated, None).scope,
            ArchitectureSourceScope::Production
        );
        assert_eq!(
            classify_source(Some("docs/reference/commands.md"), &generated, None).scope,
            ArchitectureSourceScope::Documentation
        );
        assert_eq!(
            classify_source(Some("README.md"), &generated, None).scope,
            ArchitectureSourceScope::Documentation
        );
        assert_eq!(
            classify_source(Some("packages\\web\\src\\app.ts"), &generated, None).scope,
            ArchitectureSourceScope::Production
        );
        assert_eq!(
            classify_source(Some("src/main/java/build/crab/Plan.java"), &generated, None).scope,
            ArchitectureSourceScope::Production
        );
        assert_eq!(
            classify_source(None, &generated, None).scope,
            ArchitectureSourceScope::Unknown
        );
    }
}
