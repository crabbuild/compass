#![allow(clippy::expect_used)]

use std::error::Error;
use std::fs;
use std::path::Path;

use compass_languages::{
    LanguageCapability, Registry, UNIVERSAL_EVIDENCE_SCHEMA, UniversalEvidenceQualification,
    UniversalEvidenceRegistry, file_stem, make_id, normalize_id,
};

#[test]
fn registry_covers_every_python_dispatch_extension() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let ordinary = [
        "py", "js", "jsx", "mjs", "cjs", "ts", "tsx", "mts", "cts", "go", "rs", "java", "groovy",
        "gradle", "c", "cpp", "cc", "cxx", "hpp", "cu", "cuh", "metal", "rb", "rake", "cs", "kt",
        "kts", "scala", "php", "swift", "lua", "luau", "toc", "zig", "ps1", "psm1", "psd1", "ex",
        "exs", "mm", "jl", "f", "F", "f90", "F90", "f95", "F95", "f03", "F03", "f08", "F08", "vue",
        "svelte", "astro", "dart", "v", "sv", "svh", "sql", "r", "md", "markdown", "mdx", "qmd",
        "skill", "html", "htm", "pas", "pp", "dpr", "dpk", "lpr", "inc", "dfm", "lfm", "lpk", "sh",
        "bash", "json", "tf", "tfvars", "hcl", "dm", "dme", "dmi", "dmm", "dmf", "sln", "slnx",
        "csproj", "fsproj", "vbproj", "xaml", "razor", "cshtml", "cls", "trigger", "pl", "pm",
    ];
    for extension in ordinary {
        let path = directory.path().join(format!("sample.{extension}"));
        fs::write(&path, "")?;
        assert!(Registry::resolve(&path).is_some(), "missing .{extension}");
    }
    let objc = directory.path().join("sample.m");
    fs::write(&objc, "@implementation Compass\n@end\n")?;
    assert_eq!(Registry::resolve(&objc).map(|spec| spec.name), Some("objc"));
    let header = directory.path().join("sample.h");
    fs::write(&header, "class Compass {};")?;
    assert_eq!(
        Registry::resolve(&header).map(|spec| spec.name),
        Some("cpp")
    );
    Ok(())
}

#[test]
fn every_declared_grammar_is_statically_available() {
    for grammar in [
        "apex",
        "astro",
        "bash",
        "blade",
        "c",
        "cpp",
        "csharp",
        "dart",
        "elixir",
        "fortran",
        "go",
        "groovy",
        "hcl",
        "java",
        "javascript",
        "json",
        "julia",
        "kotlin",
        "lua",
        "objc",
        "pascal",
        "perl",
        "php",
        "powershell",
        "python",
        "razor",
        "ruby",
        "rust",
        "scala",
        "sql",
        "svelte",
        "swift",
        "tsx",
        "typescript",
        "verilog",
        "vue",
        "zig",
    ] {
        assert!(
            tree_sitter_language_pack::get_language(grammar).is_ok(),
            "grammar {grammar} is not linked"
        );
    }
}

#[test]
fn ids_match_python_unicode_casefold_contract() {
    assert_eq!(normalize_id("Straße / API"), "strasse_api");
    assert_eq!(normalize_id("ＡＰＩ café"), "api_café");
    assert_eq!(normalize_id("用户/服务"), "用户_服务");
    assert_eq!(normalize_id("बनाया इतिहास"), "बन_य_इत_ह_स");
    assert_eq!(normalize_id("การติดตั้ง"), "การต_ดต_ง");
    assert_eq!(normalize_id("ref_@scope//package"), "ref_scope_package");
    assert_eq!(normalize_id("a___b"), "a_b");
    assert_eq!(
        make_id(&["src/auth/session.py", "ValidateToken"]),
        "src_auth_session_py_validatetoken"
    );
    assert_eq!(
        normalize_id(normalize_id("Straße / API").as_str()),
        "strasse_api"
    );
    assert_eq!(
        file_stem(Path::new("src/auth/session.py")),
        "src/auth/session"
    );
    assert_eq!(file_stem(Path::new("README")), "README");
    assert_eq!(file_stem(Path::new("")), "");
    assert!(Registry::resolve(Path::new("archive.zip")).is_none());
}

#[test]
fn rust_pipeline_is_version_one_and_qualified() {
    let rust = UniversalEvidenceRegistry::pipeline("rust").expect("Rust universal pipeline");
    assert_eq!(rust.producer.id, "compass.rust");
    assert_eq!(rust.producer.language, "rust");
    assert_eq!(rust.producer.evidence_schema, UNIVERSAL_EVIDENCE_SCHEMA);
    assert_eq!(rust.producer.version, 1);
    assert_eq!(
        rust.qualification,
        UniversalEvidenceQualification::Qualified
    );
    for capability in [
        LanguageCapability::Namespaces,
        LanguageCapability::Traits,
        LanguageCapability::ImplOwnership,
        LanguageCapability::Macros,
        LanguageCapability::Tests,
        LanguageCapability::Imports,
        LanguageCapability::HierarchyDispatch,
        LanguageCapability::Calls,
        LanguageCapability::ExternalReferences,
    ] {
        assert!(
            rust.producer.capabilities.contains(&capability),
            "missing {capability:?}: {rust:?}"
        );
    }

    let typescript =
        UniversalEvidenceRegistry::pipeline("typescript").expect("TypeScript universal pipeline");
    assert_eq!(typescript.producer.id, "compass.typescript");
    assert_eq!(typescript.producer.language, "typescript");
    assert_eq!(
        typescript.producer.evidence_schema,
        UNIVERSAL_EVIDENCE_SCHEMA
    );
    assert_eq!(typescript.producer.version, 1);
    assert_eq!(
        typescript.qualification,
        UniversalEvidenceQualification::Qualified
    );
    for capability in [
        LanguageCapability::Namespaces,
        LanguageCapability::Imports,
        LanguageCapability::Reexports,
        LanguageCapability::Calls,
        LanguageCapability::Construction,
        LanguageCapability::TypeReferences,
        LanguageCapability::BaseTypes,
        LanguageCapability::Members,
        LanguageCapability::ExternalReferences,
    ] {
        assert!(
            typescript.producer.capabilities.contains(&capability),
            "missing {capability:?}: {typescript:?}"
        );
    }

    let javascript =
        UniversalEvidenceRegistry::pipeline("javascript").expect("JavaScript universal pipeline");
    assert_eq!(javascript.producer.id, "compass.javascript");
    assert_eq!(javascript.producer.language, "javascript");
    assert_eq!(javascript.producer.version, 1);
    assert_eq!(
        javascript.qualification,
        UniversalEvidenceQualification::Qualified
    );
}

#[test]
fn java_pipeline_is_version_one_and_qualified() {
    let java = UniversalEvidenceRegistry::pipeline("java").expect("Java universal pipeline");
    assert_eq!(java.producer.id, "compass.java");
    assert_eq!(java.producer.language, "java");
    assert_eq!(java.producer.evidence_schema, UNIVERSAL_EVIDENCE_SCHEMA);
    assert_eq!(java.producer.version, 1);
    assert_eq!(
        java.qualification,
        UniversalEvidenceQualification::Qualified
    );
    for capability in [
        LanguageCapability::Namespaces,
        LanguageCapability::Imports,
        LanguageCapability::Calls,
        LanguageCapability::Construction,
        LanguageCapability::Decorators,
        LanguageCapability::TypeReferences,
        LanguageCapability::BaseTypes,
        LanguageCapability::Members,
        LanguageCapability::ExternalReferences,
    ] {
        assert!(
            java.producer.capabilities.contains(&capability),
            "missing {capability:?}: {java:?}"
        );
    }
}

#[test]
fn only_hard_cut_languages_expose_pipelines() {
    let python = Registry::resolve(Path::new("src/example.py")).expect("python spec");
    let go = Registry::resolve(Path::new("src/example.go")).expect("go spec");
    let java = Registry::resolve(Path::new("src/Example.java")).expect("java spec");
    let kotlin = Registry::resolve(Path::new("src/Example.kt")).expect("kotlin spec");
    let ruby = Registry::resolve(Path::new("src/example.rb")).expect("ruby spec");
    let rust = Registry::resolve(Path::new("src/example.rs")).expect("rust spec");
    let typescript = Registry::resolve(Path::new("src/example.ts")).expect("typescript spec");
    let tsx = Registry::resolve(Path::new("src/example.tsx")).expect("tsx spec");
    let javascript = Registry::resolve(Path::new("src/example.js")).expect("javascript spec");

    assert_eq!(
        Registry::universal_evidence_pipeline_for_spec(python)
            .map(|pipeline| pipeline.producer.language),
        Some("python")
    );
    assert_eq!(
        Registry::universal_evidence_pipeline_for_spec(go)
            .map(|pipeline| pipeline.producer.language),
        Some("go")
    );
    assert_eq!(
        Registry::universal_evidence_pipeline_for_spec(java)
            .map(|pipeline| pipeline.producer.language),
        Some("java")
    );
    assert_eq!(
        Registry::universal_evidence_pipeline_for_spec(kotlin)
            .map(|pipeline| pipeline.producer.language),
        Some("kotlin")
    );
    assert_eq!(
        Registry::universal_evidence_pipeline_for_spec(ruby)
            .map(|pipeline| pipeline.producer.language),
        Some("ruby")
    );
    assert_eq!(
        Registry::universal_evidence_pipeline_for_spec(rust)
            .map(|pipeline| pipeline.producer.language),
        Some("rust")
    );
    assert_eq!(
        Registry::universal_evidence_pipeline_for_spec(typescript)
            .map(|pipeline| pipeline.producer.language),
        Some("typescript")
    );
    assert_eq!(
        Registry::universal_evidence_pipeline_for_spec(tsx)
            .map(|pipeline| pipeline.producer.language),
        Some("typescript")
    );
    assert_eq!(
        Registry::universal_evidence_pipeline_for_spec(javascript)
            .map(|pipeline| pipeline.producer.language),
        Some("javascript")
    );
    assert_eq!(
        Registry::universal_evidence_pipeline(Path::new("src/example.py"))
            .map(|pipeline| pipeline.producer.language),
        Some("python")
    );
    assert_eq!(
        Registry::universal_evidence_pipeline(Path::new("src/example.tsx"))
            .map(|pipeline| pipeline.producer.language),
        Some("typescript")
    );
}
