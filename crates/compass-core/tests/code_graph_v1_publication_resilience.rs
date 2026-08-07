use std::collections::BTreeSet;
use std::error::Error;
use std::fs;
use std::path::Path;

use compass_core::{BuildOptions, build_graph_with_layers, build_local_graph};
use compass_files::{AST_CACHE_VERSION, Cache, CacheOptions};
use compass_languages::{Extraction, Registry};
use compass_model::code_graph::{CoverageStatus, ExtractionStatus, GraphDocument, NodeKind};
use compass_model::provenance::EvidenceOrigin;
use compass_model::validate_code_graph;
use sha2::{Digest, Sha256};

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
fn invalid_topology_is_quarantined_and_the_valid_graph_is_published() -> Result<(), Box<dyn Error>>
{
    let directory = tempfile::tempdir()?;
    write(directory.path(), "src/main.rs", "pub fn healthy() {}\n")?;
    let mut options = BuildOptions::new(directory.path());
    options.no_viz = true;
    options.built_at_commit = Some("0123456789012345678901234567890123456789".to_owned());
    let _first = build_local_graph(&options)?;
    let pointer = directory.path().join("compass-out/current-snapshot");
    let active_before = fs::read_to_string(&pointer)?;

    options.force = true;
    let invalid_layer = serde_json::json!({
        "nodes": [
            {
                "id": "invalid_owner",
                "label": "owner",
                "symbol_kind": "variable",
                "file_type": "code",
                "source_file": "src/main.rs",
                "source_location": "L1",
                "language": "rust",
                "extractor": "test.invalid"
            },
            {
                "id": "invalid_method",
                "label": ".method()",
                "symbol_kind": "method",
                "file_type": "code",
                "source_file": "src/main.rs",
                "source_location": "L1",
                "language": "rust",
                "extractor": "test.invalid"
            }
        ],
        "edges": [{
            "source": "invalid_owner",
            "target": "invalid_method",
            "relation": "method",
            "source_file": "src/main.rs",
            "source_location": "L1",
            "confidence": "EXTRACTED",
            "extractor": "test.invalid"
        }]
    });
    let result = build_graph_with_layers(&options, None, &[invalid_layer])?;
    assert!(result.partial_graph);
    assert_eq!(result.omitted_nodes, 0);
    assert_eq!(result.omitted_edges, 1);
    assert_eq!(result.identity_collisions, 0);
    assert_ne!(fs::read_to_string(&pointer)?, active_before);

    let graph = GraphDocument::load(&result.output_dir.join("graph.json"))?;
    validate_code_graph(&graph)?;
    let owner = graph
        .nodes
        .iter()
        .find(|node| node.qualified_name == "owner")
        .ok_or("missing retained owner")?;
    let method = graph
        .nodes
        .iter()
        .find(|node| node.qualified_name == ".method()")
        .ok_or("missing retained method")?;
    assert!(graph.links.iter().all(|edge| {
        !(edge.source == owner.id && edge.target == method.id && edge.kind.as_str() == "contains")
    }));
    assert!(graph.graph.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "publication_omission_summary"
            && diagnostic.message.contains("0 nodes and 1 edges")
    }));

    let stats: serde_json::Value =
        serde_json::from_slice(&fs::read(result.output_dir.join("output-stats.json"))?)?;
    assert_eq!(stats["omitted_nodes"], 0);
    assert_eq!(stats["omitted_edges"], 1);
    assert_eq!(stats["identity_collisions"], 0);
    Ok(())
}

#[test]
fn zero_byte_registered_source_is_truthful_inventory() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    write(directory.path(), "src/healthy.rs", "pub fn healthy() {}\n")?;
    write(directory.path(), "src/empty.rs", "")?;
    write(directory.path(), "config/Empty.csproj", "")?;
    write(directory.path(), "ui/Empty.xaml", "")?;
    write(directory.path(), "config/empty.json", "")?;

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
    let empty_nodes = graph
        .nodes
        .iter()
        .filter(|node| node.source_file() == Some("src/empty.rs"))
        .collect::<Vec<_>>();
    assert_eq!(empty_nodes.len(), 1);
    assert_eq!(empty_nodes[0].kind, NodeKind::File);
    assert!(
        graph
            .links
            .iter()
            .all(|edge| edge.source != empty_nodes[0].id && edge.target != empty_nodes[0].id),
        "zero-byte input must not invent body relationships"
    );
    for (path, language) in [
        ("config/Empty.csproj", "project-xml"),
        ("ui/Empty.xaml", "xaml"),
        ("config/empty.json", "json"),
    ] {
        let failed = graph
            .graph
            .files
            .iter()
            .find(|file| file.path == path)
            .ok_or_else(|| format!("missing empty-file inventory for {path}"))?;
        assert_eq!(failed.language.as_deref(), Some(language));
        assert_eq!(failed.byte_size, 0);
        assert_eq!(failed.extraction_status, ExtractionStatus::ParseFailure);
        assert!(graph.graph.coverage.iter().any(|coverage| {
            coverage.file_id.as_deref() == Some(failed.id.as_str())
                && coverage.status == CoverageStatus::Failed
        }));
        assert!(graph.graph.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "extractor_failure" && diagnostic.message.contains(path)
        }));
        assert!(graph.nodes.iter().all(|node| {
            node.source
                .as_ref()
                .is_none_or(|anchor| anchor.file != path)
        }));
        assert!(graph.links.iter().all(|edge| {
            edge.relationship_site
                .as_ref()
                .is_none_or(|anchor| anchor.file != path)
        }));
    }
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
            && diagnostic
                .anchor
                .as_ref()
                .is_some_and(|anchor| anchor.file == "src/App/App.csproj")
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
fn typescript_type_star_reexport_keeps_barrel_file_exact() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    write(
        directory.path(),
        "src/value.ts",
        "export const value = 1;\n",
    )?;
    write(
        directory.path(),
        "src/types.ts",
        "export interface Value {}\n",
    )?;
    write(
        directory.path(),
        "src/index.ts",
        "export * from './value.ts';\nexport type * from './types.ts';\n",
    )?;

    let graph = build(directory.path())?;
    let barrel = graph
        .graph
        .files
        .iter()
        .find(|file| file.path == "src/index.ts")
        .ok_or("missing TypeScript barrel input")?;
    assert_eq!(barrel.extraction_status, ExtractionStatus::Extracted);
    assert_eq!(
        graph
            .links
            .iter()
            .filter(|edge| {
                edge.kind.as_str() == "exports"
                    && edge.relationship_site.as_ref().is_some_and(|site| {
                        site.file == "src/index.ts" && matches!(site.start_line, 1 | 2)
                    })
            })
            .count(),
        2
    );
    Ok(())
}

#[test]
fn typescript_interface_heritage_resolves_to_the_imported_definition() -> Result<(), Box<dyn Error>>
{
    let directory = tempfile::tempdir()?;
    write(
        directory.path(),
        "src/types.ts",
        "export interface ContextOptions<T> {}\n",
    )?;
    write(
        directory.path(),
        "src/add.ts",
        "import type { ContextOptions as BaseOptions } from './types.ts';\nexport interface AddOptions<T> extends BaseOptions<T> {}\n",
    )?;

    let graph = build(directory.path())?;
    let add_options = graph
        .nodes
        .iter()
        .find(|node| {
            node.name == "AddOptions"
                && node.kind == NodeKind::Interface
                && node.source_file() == Some("src/add.ts")
        })
        .ok_or("missing AddOptions interface")?;
    let context_options = graph
        .nodes
        .iter()
        .find(|node| {
            node.name == "ContextOptions"
                && node.kind == NodeKind::Interface
                && node.source_file() == Some("src/types.ts")
        })
        .ok_or("missing ContextOptions interface")?;
    assert!(graph.links.iter().any(|edge| {
        edge.source == add_options.id
            && edge.target == context_options.id
            && edge.kind.as_str() == "extends"
            && edge
                .relationship_site
                .as_ref()
                .is_some_and(|site| site.file == "src/add.ts" && site.start_line == 2)
    }));
    Ok(())
}

#[test]
fn legacy_markerless_ast_cache_is_reextracted_conservatively() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("src/recovered.rs");
    write(
        directory.path(),
        "src/recovered.rs",
        "pub fn recovered( { missing();\n",
    )?;

    let mut cache = Cache::open(directory.path(), CacheOptions::output_directory(None))?;
    cache.save_portable_ast_batch(&[(source, Extraction::default())])?;
    drop(cache);
    let ast_root = directory.path().join("compass-out/cache/ast");
    fs::rename(
        ast_root.join(format!("v{AST_CACHE_VERSION}")),
        ast_root.join("v1"),
    )?;

    let graph = build(directory.path())?;
    let recovered = graph
        .graph
        .files
        .iter()
        .find(|file| file.path == "src/recovered.rs")
        .ok_or("missing recovered input")?;
    assert_eq!(recovered.extraction_status, ExtractionStatus::Partial);
    assert!(graph.graph.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "parser_recovery" && diagnostic.message.contains("src/recovered.rs")
    }));
    assert!(!ast_root.join("v1").exists());
    Ok(())
}

#[test]
fn sealed_legacy_build_state_cannot_skip_current_publication() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    write(directory.path(), "src/main.rs", "pub fn current() {}\n")?;
    let mut options = BuildOptions::new(directory.path());
    options.no_cluster = true;
    options.no_viz = true;
    options.built_at_commit = Some("0123456789012345678901234567890123456789".to_owned());
    let first = build_local_graph(&options)?;
    let graph_path = first.output_dir.join("graph.json");
    let state_path = first.output_dir.join("build-state.json");

    let mut graph: serde_json::Value = serde_json::from_slice(&fs::read(&graph_path)?)?;
    let files = graph["graph"]["files"]
        .as_array_mut()
        .ok_or("graph files are not an array")?;
    let main = files
        .iter_mut()
        .find(|file| file["path"] == "src/main.rs")
        .ok_or("missing main inventory")?;
    main["language"] = serde_json::Value::String("legacy-poison".to_owned());
    let graph_bytes = serde_json::to_vec_pretty(&graph)?;
    fs::write(&graph_path, &graph_bytes)?;

    let mut state: serde_json::Value = serde_json::from_slice(&fs::read(&state_path)?)?;
    state["producer"] = serde_json::Value::String("legacy-builder".to_owned());
    state["graph"]["bytes"] = serde_json::Value::from(graph_bytes.len());
    state["graph"]["sha256"] =
        serde_json::Value::String(format!("{:x}", Sha256::digest(&graph_bytes)));
    fs::write(&state_path, serde_json::to_vec_pretty(&state)?)?;

    let second = build_local_graph(&options)?;
    let graph = GraphDocument::load(&second.output_dir.join("graph.json"))?;
    let main = graph
        .graph
        .files
        .iter()
        .find(|file| file.path == "src/main.rs")
        .ok_or("missing rebuilt main inventory")?;
    assert_eq!(main.language.as_deref(), Some("rust"));
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
fn real_framework_limit_failure_isolated_from_healthy_file() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    write(
        directory.path(),
        "src/healthy.rs",
        "pub fn target() {}\npub fn healthy() { target(); }\n",
    )?;
    let overflow = "GET /overflow controllers.Example.view()\n".repeat(100_001);
    write(directory.path(), "conf/routes", &overflow)?;

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
        .find(|file| file.path == "conf/routes")
        .ok_or("missing framework-limit inventory")?;
    assert_eq!(failed.extraction_status, ExtractionStatus::ParseFailure);
    assert!(graph.graph.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "extractor_failure"
            && diagnostic.message.contains("conf/routes")
            && diagnostic.message.contains("max_facts_per_file")
    }));
    assert!(graph.links.iter().all(|edge| {
        edge.relationship_site
            .as_ref()
            .is_none_or(|anchor| anchor.file != "conf/routes")
    }));
    Ok(())
}

#[test]
fn oversized_source_is_not_parsed_and_publishes_explicit_partial_coverage()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    write(
        directory.path(),
        "src/healthy.rs",
        "pub fn target() {}\npub fn healthy() { target(); }\n",
    )?;
    write(
        directory.path(),
        "src/generated.rs",
        &"pub fn generated() {}\n".repeat(64),
    )?;

    let mut options = BuildOptions::new(directory.path());
    options.no_cluster = true;
    options.no_viz = true;
    options.program_analysis = true;
    options.max_source_bytes = 64;
    options.built_at_commit = Some("0123456789012345678901234567890123456789".to_owned());
    let result = build_local_graph(&options)?;
    let graph = GraphDocument::load(&result.output_dir.join("graph.json"))?;

    assert!(graph.nodes.iter().any(|node| {
        node.source
            .as_ref()
            .is_some_and(|source| source.file == "src/healthy.rs")
    }));
    let oversized = graph
        .graph
        .files
        .iter()
        .find(|file| file.path == "src/generated.rs")
        .ok_or("missing oversized-file inventory")?;
    assert_eq!(oversized.extraction_status, ExtractionStatus::Partial);
    assert!(graph.graph.coverage.iter().any(|coverage| {
        coverage.file_id.as_deref() == Some(oversized.id.as_str())
            && coverage.status == CoverageStatus::Partial
    }));
    assert!(graph.graph.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "partial_extraction"
            && diagnostic.message.contains("src/generated.rs")
            && diagnostic
                .message
                .contains("configured 64 byte extraction limit")
    }));
    assert!(graph.nodes.iter().all(|node| {
        node.source
            .as_ref()
            .is_none_or(|source| source.file != "src/generated.rs")
    }));
    assert!(graph.links.iter().all(|edge| {
        edge.relationship_site
            .as_ref()
            .is_none_or(|anchor| anchor.file != "src/generated.rs")
    }));
    let program: serde_json::Value =
        serde_json::from_slice(&fs::read(result.output_dir.join("program.json"))?)?;
    assert!(
        program["program"]["modules"]
            .as_array()
            .is_some_and(|modules| {
                modules
                    .iter()
                    .all(|module| module["source_file"] != "src/generated.rs")
            })
    );
    Ok(())
}

#[test]
fn every_registered_extractor_family_crosses_production_v1_publication()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    assert_eq!(Registry::cases().len(), 66, "registry case count changed");
    for case in Registry::cases() {
        write(root, case.fixture_path, case.fixture_source)?;
        let resolved = Registry::resolve(&root.join(case.fixture_path))
            .ok_or_else(|| format!("registry case {} did not resolve", case.id))?;
        assert_eq!(resolved, case.spec, "registry case {} drifted", case.id);
    }

    let extractor_kinds = Registry::cases()
        .iter()
        .map(|case| format!("{:?}", case.spec.kind))
        .collect::<BTreeSet<_>>();
    for expected in [
        "Generic",
        "Markdown",
        "Html",
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

    let graph = build(root)?;

    for case in Registry::cases() {
        let language = case.spec.name;
        let relative = case.fixture_path;
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
        let has_typed_fact = graph.nodes.iter().any(|node| {
            node.source
                .as_ref()
                .is_some_and(|anchor| anchor.file == relative)
        }) || graph.links.iter().any(|edge| {
            edge.relationship_site
                .as_ref()
                .is_some_and(|anchor| anchor.file == relative)
        });
        let has_explicit_failure = graph
            .graph
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains(relative));
        assert!(
            has_typed_fact || has_explicit_failure,
            "registry case {} published neither a typed fact nor an explicit failure",
            case.id
        );
    }
    Ok(())
}
