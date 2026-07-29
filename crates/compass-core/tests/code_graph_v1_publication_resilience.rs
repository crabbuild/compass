use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;
use std::path::Path;

use compass_core::{BuildOptions, build_local_graph};
use compass_languages::Registry;
use compass_model::code_graph::{CoverageStatus, ExtractionStatus, GraphDocument};
use compass_model::provenance::EvidenceOrigin;

fn build(root: &Path) -> Result<GraphDocument, Box<dyn Error>> {
    let mut options = BuildOptions::new(root);
    options.no_cluster = true;
    options.no_viz = true;
    options.max_workers = Some(2);
    options.built_at_commit = Some("0123456789012345678901234567890123456789".to_owned());
    let result = build_local_graph(&options)?;
    Ok(GraphDocument::load(&result.output_dir.join("graph.json"))?)
}

fn write(root: &Path, relative: &str, source: &str) -> Result<(), Box<dyn Error>> {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, source)?;
    Ok(())
}

#[test]
fn zero_byte_registered_source_is_truthful_inventory() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    write(directory.path(), "src/healthy.rs", "pub fn healthy() {}\n")?;
    write(directory.path(), "src/empty.rs", "")?;

    let graph = build(directory.path())?;
    let empty = graph
        .graph
        .files
        .iter()
        .find(|file| file.path == "src/empty.rs")
        .ok_or("missing empty-file inventory")?;
    assert_eq!(empty.language.as_deref(), Some("rust"));
    assert_eq!(empty.byte_size, 0);
    assert_eq!(empty.extraction_status, ExtractionStatus::Extracted);
    assert!(graph.graph.coverage.iter().any(|coverage| {
        coverage.file_id.as_deref() == Some(empty.id.as_str())
            && coverage.capability == "file_inventory"
            && coverage.producer == "compass.languages.rust"
            && coverage.status == CoverageStatus::Complete
    }));
    assert!(
        graph.nodes.iter().all(|node| {
            node.source
                .as_ref()
                .is_none_or(|anchor| anchor.file != "src/empty.rs")
        }),
        "zero-byte input invented a non-empty AST anchor"
    );
    Ok(())
}

#[test]
fn missing_dotnet_references_are_external_and_do_not_abort() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    write(directory.path(), "src/healthy.rs", "pub fn healthy() {}\n")?;
    write(
        directory.path(),
        "src/App/App.csproj",
        r#"<Project Sdk="Microsoft.NET.Sdk">
  <ItemGroup>
    <ProjectReference Include="../Lib/Lib.csproj" />
    <ProjectReference Include="../Missing/Missing.csproj" />
  </ItemGroup>
</Project>
"#,
    )?;
    write(
        directory.path(),
        "src/Lib/Lib.csproj",
        r#"<Project Sdk="Microsoft.NET.Sdk"></Project>"#,
    )?;

    let graph = build(directory.path())?;
    let paths = graph
        .graph
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<Vec<_>>();
    assert!(paths.contains(&"src/App/App.csproj"));
    assert!(paths.contains(&"src/Lib/Lib.csproj"));
    assert!(!paths.contains(&"src/Missing/Missing.csproj"));
    assert!(graph.graph.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "unresolved_external_reference"
            && diagnostic.message.contains("src/Missing/Missing.csproj")
            && !diagnostic
                .message
                .contains(directory.path().to_string_lossy().as_ref())
    }));
    let missing = graph
        .nodes
        .iter()
        .find(|node| node.qualified_name == "Missing.csproj")
        .ok_or("missing unresolved external identity")?;
    assert!(missing.source.is_none());
    assert!(missing.evidence.iter().any(|evidence| {
        evidence.origin == EvidenceOrigin::Heuristic
            && evidence.rule.as_deref() == Some("external-symbol-placeholder")
    }));
    Ok(())
}

#[test]
fn parser_recovery_is_partial_deterministic_and_publishes_no_exact_edges()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    write(directory.path(), "src/healthy.rs", "pub fn healthy() {}\n")?;
    write(
        directory.path(),
        "src/recovered.rs",
        "pub fn recovered( { healthy();\n",
    )?;

    let first = build(directory.path())?;
    let second = build(directory.path())?;
    let recovered = first
        .graph
        .files
        .iter()
        .find(|file| file.path == "src/recovered.rs")
        .ok_or("missing recovered input")?;
    assert_eq!(recovered.language.as_deref(), Some("rust"));
    assert_eq!(recovered.extraction_status, ExtractionStatus::Partial);
    assert!(first.graph.coverage.iter().any(|coverage| {
        coverage.file_id.as_deref() == Some(recovered.id.as_str())
            && coverage.status == CoverageStatus::Partial
            && coverage.producer == "compass.languages.rust"
    }));
    assert!(first.graph.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "parser_recovery" && diagnostic.message.contains("src/recovered.rs")
    }));
    assert!(first.links.iter().all(|edge| {
        edge.relationship_site
            .as_ref()
            .is_none_or(|anchor| anchor.file != "src/recovered.rs")
    }));
    assert_eq!(first.graph.diagnostics, second.graph.diagnostics);
    assert_eq!(first.graph.coverage, second.graph.coverage);
    Ok(())
}

#[test]
fn failed_file_isolated_from_healthy_file_and_exact_relationships() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    write(
        directory.path(),
        "src/healthy.rs",
        "pub fn target() {}\npub fn healthy() { target(); }\n",
    )?;
    write(
        directory.path(),
        "ui/Broken.xaml",
        "<Page><Button Click=\"HandleClick\"></Page>",
    )?;

    let graph = build(directory.path())?;
    assert!(graph.nodes.iter().any(|node| {
        node.source
            .as_ref()
            .is_some_and(|source| source.file == "src/healthy.rs")
    }));
    let failed = graph
        .graph
        .files
        .iter()
        .find(|file| file.path == "ui/Broken.xaml")
        .ok_or("missing failed-file inventory")?;
    assert_eq!(failed.language.as_deref(), Some("xaml"));
    assert_eq!(failed.extraction_status, ExtractionStatus::ParseFailure);
    assert!(graph.graph.coverage.iter().any(|coverage| {
        coverage.file_id.as_deref() == Some(failed.id.as_str())
            && coverage.status == CoverageStatus::Failed
            && coverage.producer == "compass.languages.xaml"
    }));
    assert!(graph.graph.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "extractor_failure"
            && diagnostic.message.contains("ui/Broken.xaml")
            && !diagnostic
                .message
                .contains(directory.path().to_string_lossy().as_ref())
    }));
    assert!(graph.links.iter().all(|edge| {
        edge.relationship_site
            .as_ref()
            .is_none_or(|anchor| anchor.file != "ui/Broken.xaml")
    }));
    Ok(())
}

#[test]
fn every_registered_extractor_family_crosses_production_v1_publication()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let mut samples = BTreeMap::new();

    for extension in Registry::supported_extensions() {
        let relative = format!("matrix/sample.{extension}");
        let resolver_seed = match *extension {
            "m" => "@implementation Matrix\n@end\n",
            "h" => "class Matrix {};\n",
            _ => " \n",
        };
        write(root, &relative, resolver_seed)?;
        let spec = Registry::resolve(&root.join(&relative))
            .ok_or_else(|| format!("supported extension .{extension} did not resolve"))?;
        samples
            .entry(spec.name)
            .or_insert((relative, format!("{:?}", spec.kind)));
    }
    for (relative, source) in [
        ("matrix/mcp.json", r#"{"mcpServers":{}}"#),
        (
            "matrix/pyproject.toml",
            "[project]\nname = \"publication-matrix\"\n",
        ),
        (
            "matrix/example.routing.yml",
            "example.route:\n  path: /matrix\n  defaults:\n    _controller: 'Example::view'\n",
        ),
        (
            "matrix/conf/routes",
            "GET /matrix controllers.Example.view()\n",
        ),
        ("matrix/example.blade.php", "<div>matrix</div>\n"),
    ] {
        write(root, relative, source)?;
        let spec = Registry::resolve(&root.join(relative))
            .ok_or_else(|| format!("special registry path {relative} did not resolve"))?;
        samples
            .entry(spec.name)
            .or_insert((relative.to_owned(), format!("{:?}", spec.kind)));
    }

    let extractor_kinds = samples
        .values()
        .map(|(_, kind)| kind.as_str())
        .collect::<BTreeSet<_>>();
    for expected in [
        "Generic",
        "Markdown",
        "JsonConfig",
        "Terraform",
        "PascalForm",
        "LazarusPackage",
        "DreamMaker",
        "Solution",
        "ProjectXml",
        "Xaml",
        "Template",
        "PackageManifest",
        "McpConfig",
        "FrameworkConfig",
    ] {
        assert!(
            extractor_kinds.contains(expected),
            "production registry has no publication sample for {expected}"
        );
    }

    for (language, (relative, _)) in &samples {
        write(root, relative, minimal_registered_source(language))?;
    }
    let graph = build(root)?;

    for (language, (relative, _)) in samples {
        let file = graph
            .graph
            .files
            .iter()
            .find(|file| file.path == relative)
            .ok_or_else(|| format!("registered {language} input was absent from inventory"))?;
        assert_eq!(
            file.language.as_deref(),
            Some(language),
            "wrong detected language for {relative}"
        );
        assert!(
            !matches!(
                file.extraction_status,
                ExtractionStatus::Unsupported | ExtractionStatus::Excluded
            ),
            "registered {language} input had no explicit extractor outcome"
        );
        let expected_status = match file.extraction_status {
            ExtractionStatus::Extracted => CoverageStatus::Complete,
            ExtractionStatus::Partial => CoverageStatus::Partial,
            ExtractionStatus::ParseFailure => CoverageStatus::Failed,
            ExtractionStatus::Generated | ExtractionStatus::Binary => CoverageStatus::Indeterminate,
            ExtractionStatus::Unsupported => CoverageStatus::Unsupported,
            ExtractionStatus::Excluded => CoverageStatus::Excluded,
        };
        assert!(graph.graph.coverage.iter().any(|coverage| {
            coverage.file_id.as_deref() == Some(file.id.as_str())
                && coverage.producer == format!("compass.languages.{language}")
                && coverage.status == expected_status
        }));
    }
    Ok(())
}

fn minimal_registered_source(language: &str) -> &'static str {
    match language {
        "json" => "{}\n",
        "terraform" => "variable \"matrix\" {}\n",
        "markdown" => "# Publication matrix\n",
        "pascal-form" => "object Form1: TForm1\nend\n",
        "lazarus-package" => "<CONFIG></CONFIG>\n",
        "dreammaker" => "world\n    name = \"matrix\"\n",
        "solution" => "Microsoft Visual Studio Solution File, Format Version 12.00\n",
        "project-xml" => "<Project />\n",
        "xaml" => "<Page />\n",
        "mcp-config" => {
            "{\"mcpServers\":{\"matrix\":{\"command\":\"npx\",\"args\":[\"@scope/matrix-mcp\"],\"env\":{\"TOKEN\":\"redacted\"}}}}\n"
        }
        "package-manifest" => "[project]\nname = \"publication-matrix\"\n",
        "drupal-routing" => {
            "example.route:\n  path: /matrix\n  defaults:\n    _controller: 'Example::view'\n"
        }
        "play-routes" => "GET /matrix controllers.Example.view()\n",
        "blade" => "@include('shared.header')\n<livewire:matrix-panel wire:click=\"refresh\" />\n",
        "sql" => "SELECT 1;\n",
        "python" => "def matrix():\n    return 1\n",
        "rust" => "pub fn matrix() {}\n",
        "javascript" | "typescript" | "tsx" => "export function matrix() {}\n",
        "go" => "package matrix\nfunc Example() {}\n",
        "cpp" => "class Matrix {};\n",
        "objc" => "@implementation Matrix\n@end\n",
        "java" | "csharp" | "kotlin" | "scala" | "apex" => "class Matrix {}\n",
        "ruby" => "def matrix\nend\n",
        "php" => "<?php function matrix() {}\n",
        "bash" => "matrix() { :; }\n",
        "pascal" => "program Matrix;\nbegin\nend.\n",
        "r" => "matrix_fn <- function() 1\n",
        _ => " \n",
    }
}
