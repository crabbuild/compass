use std::collections::BTreeSet;
use std::error::Error;
use std::path::Path;

use compass_languages::{
    DECLARATION_DOCUMENTATION_OMISSION_CODE, DeclarationDocumentationKind, Engine, EvidenceLimits,
    MAX_BATCH_DOCUMENTATION_BYTES, MAX_DECLARATION_DOCUMENTATION_BYTES, SemanticEvidenceBatch,
    UniversalEvidenceQualification, validate_evidence,
};

fn extract(file: &str, source: &str) -> Result<SemanticEvidenceBatch, Box<dyn Error>> {
    let extraction = Engine::default()
        .extract_source_combined(Path::new(file), file, source.as_bytes())?
        .graph;
    if let Some(error) = extraction.error {
        return Err(error.into());
    }
    let batch = extraction
        .semantic_evidence
        .ok_or("missing direct evidence")?;
    validate_evidence(&batch, EvidenceLimits::default())?;
    Ok(batch)
}

fn assert_documentation(
    batch: &SemanticEvidenceBatch,
    name: &str,
    source: &str,
    expected: &str,
    kind: DeclarationDocumentationKind,
) -> Result<(), Box<dyn Error>> {
    let declaration = batch
        .declarations
        .iter()
        .find(|declaration| {
            declaration.name == name
                && matches!(
                    declaration.kind.as_str(),
                    "function"
                        | "method"
                        | "class"
                        | "struct"
                        | "interface"
                        | "trait"
                        | "enum"
                        | "type_alias"
                )
        })
        .ok_or_else(|| format!("missing declaration {name}"))?;
    let documentation = declaration
        .documentation
        .as_ref()
        .ok_or_else(|| format!("missing documentation for {name}"))?;
    assert_eq!(documentation.kind, kind);
    assert_eq!(documentation.content, expected);
    let start = usize::try_from(documentation.range.start_byte)?;
    let end = usize::try_from(documentation.range.end_byte)?;
    assert_eq!(source.get(start..end), Some(expected));
    assert!(documentation.range.end_byte <= declaration.range.start_byte);
    assert_eq!(
        documentation.range.source_file,
        declaration.range.source_file
    );
    Ok(())
}

#[test]
fn rust_outer_documentation_follows_ast_markers_and_attributes() -> Result<(), Box<dyn Error>> {
    for (source, expected) in [
        (
            "/// Preserve error context.\n#[inline]\npub fn work() {}\n",
            "/// Preserve error context.\n",
        ),
        (
            "#[inline]\n/// Preserve error context.\npub fn work() {}\n",
            "/// Preserve error context.\n",
        ),
        ("/// café\r\npub fn work() {}\r\n", "/// café\r\n"),
        (
            "/** Preserve error context. */\npub fn work() {}\n",
            "/** Preserve error context. */",
        ),
        (
            "    /** café\r\n     * 文 */\r\n    pub fn work() {}\r\n",
            "/** café\r\n     * 文 */",
        ),
    ] {
        let batch = extract("src/sample.rs", source)?;
        assert_documentation(
            &batch,
            "work",
            source,
            expected,
            DeclarationDocumentationKind::RustOuter,
        )?;
        assert_eq!(batch.pipeline.version, 4);
        assert_eq!(
            batch.pipeline.qualification,
            UniversalEvidenceQualification::Qualifying
        );
    }
    Ok(())
}

#[test]
fn rust_inner_ordinary_and_other_owner_comments_do_not_attach() -> Result<(), Box<dyn Error>> {
    for source in [
        "//! Inner module documentation.\npub fn work() {}\n",
        "// Ordinary comment.\npub fn work() {}\n",
        "//// Ordinary slash comment.\npub fn work() {}\n",
        "/// First declaration.\nfn first() {}\npub fn work() {}\n",
        "struct Store;\n/// Implementation documentation.\nimpl Store { fn work() {} }\n",
    ] {
        let batch = extract("src/sample.rs", source)?;
        let declaration = batch
            .declarations
            .iter()
            .find(|declaration| declaration.name == "work")
            .ok_or("missing work declaration")?;
        assert!(declaration.documentation.is_none(), "{source}");
    }
    Ok(())
}

#[test]
fn go_leading_groups_attach_to_exact_function_method_and_single_type() -> Result<(), Box<dyn Error>>
{
    let source = "package sample\n\n// Store keeps records.\ntype Store struct{}\n\n// Read preserves errors.\n// It retains their context.\nfunc (s Store) Read() {}\n\n/* Flush sends records. */\nfunc Flush() {}\n";
    let batch = extract("sample.go", source)?;
    for (name, expected) in [
        ("Store", "// Store keeps records."),
        (
            "Read",
            "// Read preserves errors.\n// It retains their context.",
        ),
        ("Flush", "/* Flush sends records. */"),
    ] {
        assert_documentation(
            &batch,
            name,
            source,
            expected,
            DeclarationDocumentationKind::GoLeading,
        )?;
    }
    assert_eq!(batch.pipeline.version, 2);
    assert_eq!(
        batch.pipeline.qualification,
        UniversalEvidenceQualification::Qualifying
    );
    Ok(())
}

#[test]
fn go_blank_trailing_and_ambiguous_group_comments_do_not_attach() -> Result<(), Box<dyn Error>> {
    for source in [
        "package sample\n// Detached documentation.\n\nfunc Work() {}\n",
        "package sample\nfunc First() {} // Trailing comment.\nfunc Work() {}\n",
    ] {
        let batch = extract("sample.go", source)?;
        let work = batch
            .declarations
            .iter()
            .find(|declaration| declaration.name == "Work")
            .ok_or("missing Work declaration")?;
        assert!(work.documentation.is_none());
    }
    let batch = extract(
        "sample.go",
        "package sample\n// Shared group description.\ntype (\n First struct{}\n Second struct{}\n)\n",
    )?;
    for declaration in batch
        .declarations
        .iter()
        .filter(|declaration| matches!(declaration.name.as_str(), "First" | "Second"))
    {
        assert!(declaration.documentation.is_none());
    }
    Ok(())
}

#[test]
fn typescript_exports_decorators_getters_and_setters_keep_separate_owners()
-> Result<(), Box<dyn Error>> {
    let source = "declare function decorate(value: unknown): any;\n/** Work preserves errors. */\nexport async function work(): Promise<void> {}\n/** Widget retains state. */\n@decorate\nexport class Widget {\n /** Getter reads state. */\n get value(): number { return 1; }\n /** Setter writes state. */\n set value(input: number) {}\n}\n";
    let batch = extract("src/sample.ts", source)?;
    assert_documentation(
        &batch,
        "work",
        source,
        "/** Work preserves errors. */",
        DeclarationDocumentationKind::TypeScriptJsDoc,
    )?;
    assert_documentation(
        &batch,
        "Widget",
        source,
        "/** Widget retains state. */",
        DeclarationDocumentationKind::TypeScriptJsDoc,
    )?;
    let accessors = batch
        .declarations
        .iter()
        .filter(|declaration| declaration.name == "value")
        .filter_map(|declaration| declaration.documentation.as_ref())
        .map(|documentation| documentation.content.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        accessors,
        BTreeSet::from(["/** Getter reads state. */", "/** Setter writes state. */"])
    );
    assert!(
        batch
            .declarations
            .iter()
            .filter(|declaration| declaration.kind == "export")
            .all(|declaration| declaration.documentation.is_none())
    );
    assert_eq!(batch.pipeline.version, 4);
    assert_eq!(
        batch.pipeline.qualification,
        UniversalEvidenceQualification::Qualifying
    );
    Ok(())
}

#[test]
fn typescript_overloads_and_tsx_preserve_each_exact_document_range() -> Result<(), Box<dyn Error>> {
    let source = "/** Number overload. */\nexport function convert(input: number): number;\n/** String overload. */\nexport function convert(input: string): string;\n/** Runtime implementation. */\nexport function convert(input: unknown): unknown { return input; }\n";
    let batch = extract("src/sample.ts", source)?;
    let declarations = batch
        .declarations
        .iter()
        .filter(|declaration| declaration.name == "convert" && declaration.kind == "function")
        .collect::<Vec<_>>();
    assert_eq!(declarations.len(), 3);
    let mut ranges = BTreeSet::new();
    for declaration in declarations {
        let documentation = declaration
            .documentation
            .as_ref()
            .ok_or("missing overload docs")?;
        assert!(ranges.insert(documentation.range.start_byte));
        assert_eq!(
            source.get(
                usize::try_from(documentation.range.start_byte)?
                    ..usize::try_from(documentation.range.end_byte)?
            ),
            Some(documentation.content.as_str())
        );
    }
    let source =
        "/** Renders a label. */\nexport function Label() { return <span>label</span>; }\n";
    let batch = extract("src/sample.tsx", source)?;
    assert_documentation(
        &batch,
        "Label",
        source,
        "/** Renders a label. */",
        DeclarationDocumentationKind::TypeScriptJsDoc,
    )?;
    Ok(())
}

#[test]
fn ordinary_typescript_comments_and_javascript_emit_no_new_documentation()
-> Result<(), Box<dyn Error>> {
    for source in [
        "// Ordinary comment.\nexport function work() {}\n",
        "/* Ordinary block. */\nexport function work() {}\n",
    ] {
        let batch = extract("src/sample.ts", source)?;
        assert!(
            batch
                .declarations
                .iter()
                .all(|declaration| declaration.documentation.is_none())
        );
    }
    let batch = extract(
        "src/sample.js",
        "/** Existing JavaScript source. */\nexport function work() {}\n",
    )?;
    assert!(
        batch
            .declarations
            .iter()
            .all(|declaration| declaration.documentation.is_none())
    );
    assert_eq!(batch.pipeline.version, 3);
    assert_eq!(
        batch.pipeline.qualification,
        UniversalEvidenceQualification::Qualifying
    );
    Ok(())
}

#[test]
fn oversized_and_interleaved_documentation_preserves_structural_facts_with_warning()
-> Result<(), Box<dyn Error>> {
    for source in [
        format!(
            "/** {} */\npub fn work() {{}}\n",
            "x".repeat(MAX_DECLARATION_DOCUMENTATION_BYTES)
        ),
        "/// First group.\n#[inline]\n/// Second group.\npub fn work() {}\n".to_owned(),
    ] {
        let batch = extract("src/sample.rs", &source)?;
        let work = batch
            .declarations
            .iter()
            .find(|declaration| declaration.name == "work")
            .ok_or("bounded docs lost the declaration")?;
        assert!(work.documentation.is_none());
        assert_eq!(
            batch
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == DECLARATION_DOCUMENTATION_OMISSION_CODE)
                .count(),
            1
        );
    }
    Ok(())
}

#[test]
fn aggregate_documentation_budget_retains_all_declarations_and_one_warning()
-> Result<(), Box<dyn Error>> {
    // Rust line-comment AST ranges include the terminating LF. Count its byte
    // as part of the exact source excerpt, so each retained group is 4096 bytes.
    let comment = format!(
        "/// {}",
        "x".repeat(MAX_DECLARATION_DOCUMENTATION_BYTES - 5)
    );
    let retained = MAX_BATCH_DOCUMENTATION_BYTES / MAX_DECLARATION_DOCUMENTATION_BYTES;
    let mut source = String::new();
    for index in 0..retained + 4 {
        source.push_str(&format!("{comment}\npub fn work_{index}() {{}}\n"));
    }
    let batch = extract("src/sample.rs", &source)?;
    assert_eq!(
        batch
            .declarations
            .iter()
            .filter(|declaration| declaration.kind == "function")
            .count(),
        retained + 4
    );
    assert_eq!(
        batch
            .declarations
            .iter()
            .filter(|declaration| declaration.documentation.is_some())
            .count(),
        retained
    );
    assert_eq!(
        batch
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == DECLARATION_DOCUMENTATION_OMISSION_CODE)
            .count(),
        1
    );
    Ok(())
}

#[test]
fn malformed_declarations_do_not_gain_leading_documentation() -> Result<(), Box<dyn Error>> {
    for (file, source) in [
        ("sample.rs", "/// Claimed behavior.\npub fn work( { }\n"),
        (
            "sample.go",
            "package sample\n// Claimed behavior.\nfunc Work( {\n",
        ),
        (
            "sample.ts",
            "/** Claimed behavior. */\nexport function work( {\n",
        ),
    ] {
        let batch = extract(file, source)?;
        assert!(
            batch
                .declarations
                .iter()
                .all(|declaration| declaration.documentation.is_none()),
            "{file}: {batch:?}"
        );
    }
    Ok(())
}
