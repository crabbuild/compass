use std::error::Error;
use std::fs;

use compass_languages::{
    CandidateRelation, EXTRACTION_QUALITY_EXTENSION, EXTRACTION_QUALITY_PARTIAL, Engine,
    ExtractError, FrameworkCapability, FrameworkLimits, FrameworkManifestPolicy,
    FrameworkOccurrencePolicy, FrameworkPackDescriptor, FrameworkPackKind, FrameworkPackRegistry,
    FrameworkPackRegistryError, FrameworkRelation, LanguageCapability, Registry, SemanticRole,
    make_id,
};

fn valid_universal_framework_pack(id: &'static str) -> FrameworkPackDescriptor {
    FrameworkPackDescriptor {
        id,
        kind: FrameworkPackKind::Source,
        languages: &["go", "python"],
        required_capabilities: &[LanguageCapability::Calls],
        framework_capabilities: &[FrameworkCapability::Messaging],
        dependency_markers: &["example/framework"],
        manifest_policy: FrameworkManifestPolicy::Required,
        activation_rules: &["decorated-handler"],
        accepted_roles: &[SemanticRole::Call],
        emitted_relation_families: &[FrameworkRelation::Handles],
        occurrence_policy: FrameworkOccurrencePolicy::ExactEvidence,
        limits: FrameworkLimits::default(),
    }
}

#[test]
fn universal_framework_pack_registry_accepts_only_cut_over_language_evidence() {
    let descriptor = valid_universal_framework_pack("example-handlers");
    assert_eq!(
        FrameworkPackRegistry::validate_descriptors(&[descriptor]),
        Ok(())
    );
    assert_eq!(FrameworkPackRegistry::descriptors().len(), 1);
    assert_eq!(FrameworkPackRegistry::descriptors()[0].id, "spring-java");
    assert_eq!(FrameworkPackRegistry::validate(), Ok(()));

    let rust = FrameworkPackDescriptor {
        languages: &["rust"],
        ..descriptor
    };
    assert_eq!(FrameworkPackRegistry::validate_descriptors(&[rust]), Ok(()));

    let java = FrameworkPackDescriptor {
        languages: &["java"],
        ..descriptor
    };
    assert_eq!(FrameworkPackRegistry::validate_descriptors(&[java]), Ok(()));

    let typescript = FrameworkPackDescriptor {
        languages: &["typescript"],
        ..descriptor
    };
    assert_eq!(
        FrameworkPackRegistry::validate_descriptors(&[typescript]),
        Err(FrameworkPackRegistryError::NonUniversalLanguage {
            pack: descriptor.id,
            language: "typescript",
        })
    );

    let unsupported = FrameworkPackDescriptor {
        languages: &["python"],
        required_capabilities: &[LanguageCapability::Calls, LanguageCapability::Receivers],
        accepted_roles: &[SemanticRole::Receiver],
        ..descriptor
    };
    assert_eq!(
        FrameworkPackRegistry::validate_descriptors(&[unsupported]),
        Err(FrameworkPackRegistryError::UnsupportedCapability {
            pack: descriptor.id,
            language: "python",
            capability: LanguageCapability::Receivers,
        })
    );
}

#[test]
fn universal_framework_pack_registry_enforces_evidence_activation_and_limits() {
    let descriptor = valid_universal_framework_pack("bounded-pack");

    let missing_role_capability = FrameworkPackDescriptor {
        accepted_roles: &[SemanticRole::Import],
        ..descriptor
    };
    assert_eq!(
        FrameworkPackRegistry::validate_descriptors(&[missing_role_capability]),
        Err(FrameworkPackRegistryError::RoleCapabilityNotDeclared {
            pack: descriptor.id,
            role: SemanticRole::Import,
        })
    );

    let missing_relation_capability = FrameworkPackDescriptor {
        emitted_relation_families: &[FrameworkRelation::RoutesTo],
        ..descriptor
    };
    assert_eq!(
        FrameworkPackRegistry::validate_descriptors(&[missing_relation_capability]),
        Err(FrameworkPackRegistryError::RelationCapabilityNotDeclared {
            pack: descriptor.id,
            relation: FrameworkRelation::RoutesTo,
        })
    );

    let missing_framework_capability = FrameworkPackDescriptor {
        framework_capabilities: &[],
        ..descriptor
    };
    assert_eq!(
        FrameworkPackRegistry::validate_descriptors(&[missing_framework_capability]),
        Err(FrameworkPackRegistryError::EmptyFrameworkCapabilities(
            descriptor.id
        ))
    );

    let missing_manifest_evidence = FrameworkPackDescriptor {
        dependency_markers: &[],
        ..descriptor
    };
    assert_eq!(
        FrameworkPackRegistry::validate_descriptors(&[missing_manifest_evidence]),
        Err(FrameworkPackRegistryError::MissingRequiredDependencyMarkers(descriptor.id))
    );

    let unnamed_heuristic = FrameworkPackDescriptor {
        activation_rules: &[],
        manifest_policy: FrameworkManifestPolicy::Advisory,
        occurrence_policy: FrameworkOccurrencePolicy::ExactAnchoredHeuristic,
        ..descriptor
    };
    assert_eq!(
        FrameworkPackRegistry::validate_descriptors(&[unnamed_heuristic]),
        Err(FrameworkPackRegistryError::MissingHeuristicRule(
            descriptor.id
        ))
    );

    let zero_limit = FrameworkPackDescriptor {
        limits: FrameworkLimits {
            max_candidates: 0,
            ..FrameworkLimits::default()
        },
        ..descriptor
    };
    assert_eq!(
        FrameworkPackRegistry::validate_descriptors(&[zero_limit]),
        Err(FrameworkPackRegistryError::ZeroLimit {
            pack: descriptor.id,
            limit: "max_candidates",
        })
    );

    assert_eq!(
        FrameworkPackRegistry::validate_descriptors(&[descriptor, descriptor]),
        Err(FrameworkPackRegistryError::DuplicateId(descriptor.id))
    );
}

#[test]
fn caller_supplied_source_matches_file_based_generic_extraction() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("source.rs");
    let source = b"pub struct Service;\nimpl Service { pub fn run(&self) {} }\n";
    fs::write(&path, source)?;

    let from_file = Engine::default().extract(&path)?;
    let from_memory = Engine::default().extract_source(&path, source)?;
    assert_eq!(from_memory, from_file);
    Ok(())
}

#[test]
fn rust_methods_include_their_declaring_impl_in_semantic_identity() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("changes.rs");
    let source = br#"
trait ChangeSink {
    fn change(&mut self);
}
struct ExactDiffWriter;
struct ChangeCounts;
impl ChangeSink for ExactDiffWriter {
    fn change(&mut self) {}
}
impl ChangeSink for ChangeCounts {
    fn change(&mut self) {}
}
"#;

    let extraction = Engine::default().extract_source(&path, source)?;
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing Rust semantic evidence")?;
    let methods = evidence
        .declarations
        .iter()
        .filter(|declaration| {
            declaration.kind == "method" && declaration.qualified_name.starts_with('<')
        })
        .collect::<Vec<_>>();

    assert_eq!(methods.len(), 2, "declarations={:?}", evidence.declarations);
    assert_eq!(
        methods
            .iter()
            .map(|declaration| declaration.qualified_name.as_str())
            .collect::<std::collections::BTreeSet<_>>(),
        [
            "<crate::changes::ChangeCounts as crate::changes::ChangeSink>::change",
            "<crate::changes::ExactDiffWriter as crate::changes::ChangeSink>::change"
        ]
        .into_iter()
        .collect()
    );
    assert!(extraction.nodes.is_empty());
    assert!(extraction.edges.is_empty());
    Ok(())
}

#[test]
fn overloaded_methods_emit_stable_universal_identities() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("Example.java");
    let source = br#"
class Example {
    void run() {}
    void run(int value) {}
}
"#;

    let extraction = Engine::default().extract_source(&path, source)?;
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing Java universal evidence")?;
    let methods = evidence
        .declarations
        .iter()
        .filter(|declaration| declaration.qualified_name.ends_with("Example::run"))
        .collect::<Vec<_>>();

    assert_eq!(methods.len(), 2, "declarations={:?}", evidence.declarations);
    assert_eq!(
        methods
            .iter()
            .map(|declaration| declaration.signature.as_deref())
            .collect::<std::collections::BTreeSet<_>>(),
        [Some("run()"), Some("run(int)")].into_iter().collect()
    );
    assert_ne!(methods[0].graph_node_id, methods[1].graph_node_id);
    assert!(extraction.nodes.is_empty() && extraction.edges.is_empty());
    Ok(())
}

#[test]
fn generic_methods_include_their_declaring_class_in_semantic_identity() -> Result<(), Box<dyn Error>>
{
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("bundle.js");
    let source = br#"
class First {
    constructor(e, t) { this.value = e + t; }
}
class Second {
    constructor(e, t) { this.value = e * t; }
}
"#;

    let extraction = Engine::default().extract_source(&path, source)?;
    let constructors = extraction
        .nodes
        .iter()
        .filter(|node| node.label() == ".constructor()")
        .collect::<Vec<_>>();

    assert_eq!(constructors.len(), 2, "nodes={:?}", extraction.nodes);
    assert_eq!(
        constructors
            .iter()
            .map(|node| node.string("lexical_owner"))
            .collect::<std::collections::BTreeSet<_>>(),
        ["First", "Second"].into_iter().map(str::to_owned).collect()
    );
    assert_eq!(
        constructors
            .iter()
            .map(|node| node.string("qualified_name"))
            .collect::<std::collections::BTreeSet<_>>(),
        ["First::constructor", "Second::constructor"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    );
    Ok(())
}

#[test]
fn php_methods_include_their_declaring_class_in_semantic_identity() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("controllers.php");
    let source = br#"<?php
class FirstController {
    public function index() {}
}
class SecondController {
    public function index() {}
}
"#;

    let extraction = Engine::default().extract_source(&path, source)?;
    let methods = extraction
        .nodes
        .iter()
        .filter(|node| node.label() == ".index()")
        .collect::<Vec<_>>();

    assert_eq!(methods.len(), 2, "nodes={:?}", extraction.nodes);
    assert_eq!(
        methods
            .iter()
            .map(|node| node.string("lexical_owner"))
            .collect::<std::collections::BTreeSet<_>>(),
        ["FirstController", "SecondController"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    );
    assert_eq!(
        methods
            .iter()
            .map(|node| node.string("qualified_name"))
            .collect::<std::collections::BTreeSet<_>>(),
        ["FirstController::index", "SecondController::index"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    );
    Ok(())
}

#[test]
fn repeated_rust_calls_keep_exact_sites_and_known_producer_metadata() -> Result<(), Box<dyn Error>>
{
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("repeated.rs");
    let source = b"fn callee(){} fn caller(){callee();callee();}";
    fs::write(&path, source)?;

    let extraction = Engine::default().extract(&path)?;
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing Rust semantic evidence")?;
    let calls = evidence
        .occurrences
        .iter()
        .filter(|occurrence| {
            occurrence.role == SemanticRole::Call && occurrence.spelling == "callee"
        })
        .collect::<Vec<_>>();

    assert_eq!(calls.len(), 2, "occurrences={:?}", evidence.occurrences);
    assert_eq!(evidence.adapter.language, "rust");
    assert_eq!(
        evidence.adapter.producer,
        "compass.languages.rust.universal"
    );

    let mut sites = calls
        .iter()
        .map(|occurrence| {
            (
                usize::try_from(occurrence.range.start_byte),
                usize::try_from(occurrence.range.end_byte),
                occurrence.range.start_line,
                occurrence.range.start_column,
                occurrence.range.end_column,
            )
        })
        .collect::<Vec<_>>();
    sites.sort_unstable_by_key(|site| site.0.as_ref().copied().unwrap_or(usize::MAX));
    assert_eq!(
        sites,
        [(Ok(26), Ok(32), 1, 26, 32), (Ok(35), Ok(41), 1, 35, 41)]
    );
    assert_eq!(&source[26..32], b"callee");
    assert_eq!(&source[35..41], b"callee");
    Ok(())
}

#[test]
fn rust_scoped_calls_do_not_bind_terminal_names_to_unrelated_symbols() -> Result<(), Box<dyn Error>>
{
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("scoped.rs");
    fs::write(
        &path,
        b"
trait Error {}
enum RejectionReason { Error(String) }
fn reject(message: String) {
    let _ = RejectionReason::Error(message);
}
",
    )?;

    let extraction = Engine::default().extract(&path)?;
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing Rust semantic evidence")?;
    let error_trait = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.kind == "trait" && declaration.name == "Error")
        .ok_or("missing Error trait")?;
    let reject = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.kind == "function" && declaration.name == "reject")
        .ok_or("missing reject function")?;

    assert!(
        !evidence.candidates.iter().any(|candidate| {
            candidate.relation == CandidateRelation::Calls
                && candidate.source_declaration_id == reject.id
                && candidate.constraints.qualified_name.as_deref()
                    == Some(error_trait.qualified_name.as_str())
        }),
        "scoped enum construction targeted unrelated trait: {:?}",
        evidence.candidates
    );
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.source_declaration_id == reject.id
            && candidate
                .constraints
                .qualified_name
                .as_deref()
                .is_some_and(|name| name.ends_with("RejectionReason::Error"))
    }));
    Ok(())
}

#[test]
fn repeated_generic_calls_keep_each_ast_range() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("repeated.py");
    let source = b"def callee(): pass\ndef caller(): callee(); callee()\n";
    fs::write(&path, source)?;

    let extraction = Engine::default().extract(&path)?;
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing Python semantic evidence")?;
    let calls = evidence
        .occurrences
        .iter()
        .filter(|occurrence| {
            occurrence.role == SemanticRole::Call && occurrence.spelling == "callee"
        })
        .collect::<Vec<_>>();
    let mut sites = calls
        .iter()
        .map(|occurrence| {
            (
                occurrence.range.start_byte,
                occurrence.range.end_byte,
                occurrence.range.start_column,
                occurrence.range.end_column,
            )
        })
        .collect::<Vec<_>>();
    sites.sort_unstable();

    assert_eq!(
        sites,
        [(33, 39, 14, 20), (43, 49, 24, 30)],
        "occurrences={:?}",
        evidence.occurrences
    );
    Ok(())
}

#[test]
fn repeated_go_calls_keep_each_ast_range() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("repeated.go");
    let source = b"package p\nfunc callee(){};func caller(){callee();callee()}\n";
    fs::write(&path, source)?;

    let extraction = Engine::default().extract(&path)?;
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing Go semantic evidence")?;
    let sites = evidence
        .occurrences
        .iter()
        .filter(|occurrence| {
            occurrence.role == SemanticRole::Call && occurrence.spelling == "callee"
        })
        .map(|occurrence| (occurrence.range.start_byte, occurrence.range.end_byte))
        .collect::<Vec<_>>();

    assert_eq!(sites.len(), 2, "occurrences={:?}", evidence.occurrences);
    assert!(sites.iter().all(|(start, end)| start < end));
    assert_ne!(sites[0], sites[1]);
    Ok(())
}

#[test]
fn bash_entrypoints_and_go_embeddings_retain_typed_exact_facts() -> Result<(), Box<dyn Error>> {
    let mut engine = Engine::default();
    let bash = engine.extract_source(
        std::path::Path::new("scripts/release.sh"),
        b"#!/usr/bin/env bash\nlog() { :; }\nlog\n",
    )?;
    let entry = bash
        .nodes
        .iter()
        .find(|node| node.label() == "release.sh script")
        .ok_or("missing Bash entrypoint")?;
    assert_eq!(entry.string("symbol_kind"), "function");
    assert!(bash.edges.iter().any(|edge| {
        edge.target == entry.id
            && edge.string("relation") == "contains"
            && edge.string("source_location") == "L1"
    }));
    assert!(bash.edges.iter().any(|edge| {
        edge.source == entry.id
            && edge.string("relation") == "calls"
            && edge.string("source_location") == "L3"
    }));

    let go = engine.extract_source(
        std::path::Path::new("service/types.go"),
        br#"package service
import "example.com/project/agent"
type Local interface { agent.Agent }
"#,
    )?;
    let evidence = go
        .semantic_evidence
        .as_ref()
        .ok_or("missing Go semantic evidence")?;
    let embedding = evidence
        .candidates
        .iter()
        .find(|candidate| candidate.relation == CandidateRelation::Embeds)
        .ok_or("missing Go embedding")?;
    let occurrence_id = embedding
        .occurrence_id
        .as_deref()
        .ok_or("embedding has no occurrence")?;
    let occurrence = evidence
        .occurrences
        .iter()
        .find(|occurrence| occurrence.id == occurrence_id)
        .ok_or("missing embedding occurrence")?;
    assert_eq!(occurrence.range.start_line, 3);
    assert_eq!(occurrence.spelling, "Agent");
    assert!(evidence.bindings.iter().any(|binding| {
        embedding.binding_id.as_deref() == Some(binding.id.as_str())
            && binding.qualified_target == "example.com/project/agent"
    }));
    assert!(embedding.constraints.allow_external);
    Ok(())
}

#[test]
fn python_decorators_are_exact_uses_not_file_wide_import_inference() -> Result<(), Box<dyn Error>> {
    let source = br#"from framework import used, unused

@used("class")
class Consumer:
    @used("method")
    def run(self):
        pass
"#;
    let extraction =
        Engine::default().extract_source(std::path::Path::new("app/consumer.py"), source)?;
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing Python semantic evidence")?;
    let uses = evidence
        .occurrences
        .iter()
        .filter(|occurrence| {
            occurrence.role == SemanticRole::Decorator && occurrence.spelling == "used"
        })
        .collect::<Vec<_>>();
    assert_eq!(uses.len(), 2, "occurrences={:#?}", evidence.occurrences);
    let mut use_lines = uses
        .iter()
        .map(|occurrence| occurrence.range.start_line)
        .collect::<Vec<_>>();
    use_lines.sort_unstable();
    assert_eq!(use_lines, [3, 5]);
    assert!(uses.iter().all(|occurrence| {
        occurrence.range.start_byte < occurrence.range.end_byte
            && evidence.candidates.iter().any(|candidate| {
                candidate.occurrence_id.as_deref() == Some(&occurrence.id)
                    && candidate.constraints.qualified_name.as_deref() == Some("framework.used")
            })
    }));
    assert!(
        evidence
            .candidates
            .iter()
            .filter(|candidate| candidate.relation == CandidateRelation::Decorates)
            .all(|candidate| {
                candidate.constraints.qualified_name.as_deref() != Some("framework.unused")
            })
    );
    Ok(())
}

#[test]
fn repeated_zig_calls_keep_each_source_range() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("repeated.zig");
    let source = b"fn callee() void {}\nfn caller() void { callee(); callee(); }\n";
    fs::write(&path, source)?;

    let extraction = Engine::default().extract(&path)?;
    let sites = extraction
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls")
        .map(|edge| {
            (
                edge.attributes
                    .get("start_byte")
                    .and_then(|value| value.as_u64()),
                edge.attributes
                    .get("end_byte")
                    .and_then(|value| value.as_u64()),
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        sites,
        [(Some(39), Some(45)), (Some(49), Some(55))],
        "edges={:?}",
        extraction.edges
    );
    Ok(())
}

#[test]
fn repeated_dart_framework_calls_keep_each_source_range() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("repeated.dart");
    let source =
        b"class State {}\nclass Controller { void run() { emit(State()); emit(State()); } }\n";
    fs::write(&path, source)?;

    let extraction = Engine::default().extract(&path)?;
    let sites = extraction
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls" && edge.string("context") == "emit_state")
        .map(|edge| {
            (
                edge.attributes
                    .get("start_byte")
                    .and_then(|value| value.as_u64()),
                edge.attributes
                    .get("end_byte")
                    .and_then(|value| value.as_u64()),
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(sites.len(), 2, "edges={:?}", extraction.edges);
    assert!(sites.iter().all(|(start, end)| start < end));
    assert_ne!(sites[0], sites[1]);
    Ok(())
}

struct NavigationSite {
    start: usize,
    end: usize,
    line: u64,
    column: u64,
}

fn dart_navigation_sites(source: &[u8]) -> Result<Vec<NavigationSite>, Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("navigation.dart");
    fs::write(&path, source)?;
    let extraction = Engine::default().extract(&path)?;
    extraction
        .edges
        .iter()
        .filter(|edge| {
            edge.string("relation") == "navigates" && edge.string("context") == "route_path"
        })
        .map(|edge| {
            let start = edge.attributes["start_byte"]
                .as_u64()
                .ok_or("missing start_byte")?;
            let end = edge.attributes["end_byte"]
                .as_u64()
                .ok_or("missing end_byte")?;
            let line = edge.attributes["line_start"]
                .as_u64()
                .ok_or("missing line_start")?;
            let column = edge.attributes["column_start"]
                .as_u64()
                .ok_or("missing column_start")?;
            Ok(NavigationSite {
                start: usize::try_from(start)?,
                end: usize::try_from(end)?,
                line,
                column,
            })
        })
        .collect()
}

#[test]
fn dart_ascii_navigation_range_slices_original_source() -> Result<(), Box<dyn Error>> {
    let source = b"void run() { go('/home'); }\n";
    let sites = dart_navigation_sites(source)?;

    assert_eq!(sites.len(), 1);
    assert_eq!(&source[sites[0].start..sites[0].end], b"go('/home'");
    assert_eq!((sites[0].line, sites[0].column), (1, 13));
    Ok(())
}

#[test]
fn dart_multiline_comment_preserves_navigation_bytes_and_lines() -> Result<(), Box<dyn Error>> {
    let source = b"/* lead\ncomment */\nvoid run() { go('/home'); }\n";
    let sites = dart_navigation_sites(source)?;

    assert_eq!(sites.len(), 1);
    assert_eq!(&source[sites[0].start..sites[0].end], b"go('/home'");
    assert_eq!((sites[0].line, sites[0].column), (3, 13));
    Ok(())
}

#[test]
fn dart_utf8_prefix_preserves_byte_based_navigation_range() -> Result<(), Box<dyn Error>> {
    let source = "const label = 'café';\nvoid run() { go('/home'); }\n".as_bytes();
    let sites = dart_navigation_sites(source)?;

    assert_eq!(sites.len(), 1);
    assert_eq!(&source[sites[0].start..sites[0].end], b"go('/home'");
    assert_eq!((sites[0].line, sites[0].column), (2, 13));
    Ok(())
}

#[test]
fn dart_minified_navigation_keeps_same_line_occurrences_distinct() -> Result<(), Box<dyn Error>> {
    let source = b"void run(){go('/a');go('/b');}\n";
    let sites = dart_navigation_sites(source)?;

    assert_eq!(sites.len(), 2);
    assert_eq!(&source[sites[0].start..sites[0].end], b"go('/a'");
    assert_eq!(&source[sites[1].start..sites[1].end], b"go('/b'");
    assert_ne!(sites[0].start, sites[1].start);
    Ok(())
}

#[test]
fn repeated_razor_component_calls_keep_each_source_range() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("Repeated.razor");
    fs::write(&path, "<Widget /><Widget />")?;

    let extraction = Engine::default().extract(&path)?;
    let sites = extraction
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls")
        .map(|edge| {
            (
                edge.attributes
                    .get("start_byte")
                    .and_then(|value| value.as_u64()),
                edge.attributes
                    .get("end_byte")
                    .and_then(|value| value.as_u64()),
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(sites, [(Some(0), Some(8)), (Some(10), Some(18))]);
    Ok(())
}

#[test]
fn extensionless_perl_shebang_extracts_subroutines_and_calls() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("buildah-vendor-treadmill");
    fs::write(
        &path,
        r#"#!/usr/bin/perl
use strict;
use warnings;

sub helper {
    return 1;
}

sub run {
    helper();
}

run();
"#,
    )?;

    assert_eq!(Registry::resolve(&path).map(|spec| spec.name), Some("perl"));
    let extraction = Engine::default().extract(&path)?;
    assert!(
        extraction
            .nodes
            .iter()
            .any(|node| node.label() == "helper()"),
        "nodes={:?}",
        extraction.nodes
    );
    assert!(
        extraction.nodes.iter().any(|node| node.label() == "run()"),
        "nodes={:?}",
        extraction.nodes
    );
    let helper_id = make_id(&[&path.to_string_lossy(), "helper"]);
    let run_id = make_id(&[&path.to_string_lossy(), "run"]);
    assert!(
        extraction.edges.iter().any(|edge| {
            edge.source == run_id && edge.target == helper_id && edge.string("relation") == "calls"
        }),
        "edges={:?}",
        extraction.edges
    );
    Ok(())
}

#[test]
fn python_indirect_rationale_types_and_binding_shapes_are_extracted() -> Result<(), Box<dyn Error>>
{
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("advanced.py");
    fs::write(
        &source,
        r#""""A module rationale long enough to become a rationale node."""
from package import imported as alias
from typing import Annotated, Callable, Generic, Optional, TypeVar, Union

T = TypeVar("T")
external_map = {"handler": external_handler}
external_list = [first_handler, second_handler]

class Base(Generic[T]):
    """A class rationale long enough to be indexed safely."""

class Service(Base[ExternalType]):
    def execute(
        self,
        callback: Callable[[InputType], OutputType],
        value: Annotated[Optional[Union[InputType, None]], "meta"],
    ) -> tuple[OutputType, ...]:
        """A function rationale long enough to be indexed safely."""
        # WHY: retain this adapter for compatibility with old callers
        local = callback
        assigned = (external_factory, local)
        consume(external_argument, named=external_keyword)
        mapping = {"one": dictionary_handler, "bound": local}
        handlers = {set_handler, local}
        alias()
        with open_resource() as resource:
            resource.use()
        for item in iterator_factory():
            item.run()
        try:
            risky()
        except ErrorType as error:
            error.handle()
        return external_result

def top_level(arg: InputType) -> OutputType:
    alias()
    return external_top
"#,
    )?;
    let mut engine = Engine::default();
    let extraction = engine.extract(&source)?;
    assert!(
        extraction
            .nodes
            .iter()
            .any(|node| node.string("file_type") == "rationale")
    );
    assert!(
        extraction
            .edges
            .iter()
            .any(|edge| edge.string("relation") == "rationale_for")
    );
    assert!(extraction.raw_calls.is_none());
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing Python semantic evidence")?;
    for callee in ["set_handler", "external_result", "external_top"] {
        assert!(
            evidence.occurrences.iter().any(|occurrence| {
                occurrence.role == compass_languages::SemanticRole::CallableReference
                    && occurrence.spelling == callee
            }),
            "missing {callee}; occurrences={:?}",
            evidence.occurrences
        );
    }
    assert!(
        evidence
            .declarations
            .iter()
            .any(|declaration| declaration.name == "Service"),
        "declarations={:?}; diagnostics={:?}",
        evidence.declarations,
        evidence.diagnostics
    );
    assert!(
        evidence
            .declarations
            .iter()
            .any(|declaration| declaration.name == "top_level"),
        "declarations={:?}; diagnostics={:?}",
        evidence.declarations,
        evidence.diagnostics
    );
    Ok(())
}

#[test]
fn generated_python_javascript_exports_and_static_type_families_cover_rare_ast_shapes()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let fixtures = [
        (
            "migration.py",
            r#""""This generated module rationale must be suppressed by migration detection."""
revision = "abc"
down_revision = "def"
def upgrade():
    """Nested rationale remains discoverable for the upgrade function."""
    pass
"#,
        ),
        (
            "module.ts",
            r#"export const handler = (value: Input): Output => factory(value);
export const mapping = { first: handler };
class Box<T extends Base> extends Parent implements Contract {
  field: Array<Item>;
  run(arg: Promise<Input>): Result<Output> { return helper(arg); }
}
"#,
        ),
        (
            "Types.kt",
            r#"enum class Mode { FAST, SAFE }
class Box<T : Base>(val item: Item) : Parent(), Contract {
    val values: List<External> = listOf()
    fun <R : Result> run(input: Input): Map<String, R> = helper(input)
}
"#,
        ),
        (
            "Types.scala",
            r#"trait Contract
class Box[T <: Base](value: Input) extends Parent with Contract {
  val field: Either[Failure, T] = ???
  def run[R](input: Option[Input]): (R, Output) = helper(input)
}
"#,
        ),
        (
            "Types.java",
            r#"enum Mode { FAST(1), SAFE(2); Mode(int n) {} }
class Box<T extends Base> extends Parent implements Contract {
  java.util.List<Item> field;
  <R extends Result> R run(Input input) { return helper(input); }
}
"#,
        ),
        (
            "types.c",
            r#"typedef struct Payload Payload;
struct Box { Payload *payload; };
Result run(const Input *input, Output **output) { return helper(input); }
"#,
        ),
    ];
    let mut engine = Engine::default();
    for (name, text) in fixtures {
        let path = directory.path().join(name);
        fs::write(&path, text)?;
        let extraction = engine.extract(&path)?;
        assert!(
            !extraction.nodes.is_empty()
                || extraction
                    .semantic_evidence
                    .as_ref()
                    .is_some_and(|evidence| !evidence.declarations.is_empty()),
            "{name}"
        );
    }
    let migration = engine.extract(&directory.path().join("migration.py"))?;
    assert!(
        !migration
            .nodes
            .iter()
            .any(|node| node.label().contains("generated module"))
    );
    let migration_evidence = migration
        .semantic_evidence
        .as_ref()
        .ok_or("missing migration Python evidence")?;
    assert!(
        migration_evidence
            .declarations
            .iter()
            .any(|declaration| declaration.name == "upgrade"),
        "declarations={:?}; diagnostics={:?}",
        migration_evidence.declarations,
        migration_evidence.diagnostics
    );

    let missing = directory.path().join("missing.py");
    assert!(matches!(
        engine.extract(&missing),
        Err(ExtractError::File(_))
    ));
    let unsupported = directory.path().join("unsupported.unknown");
    fs::write(&unsupported, "data")?;
    assert!(matches!(
        engine.extract(&unsupported),
        Err(ExtractError::Unsupported(_))
    ));
    Ok(())
}

#[test]
fn typescript_import_resolution_checks_extensions_and_directory_indexes()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    fs::create_dir_all(directory.path().join("pkg"))?;
    fs::write(
        directory.path().join("target.ts"),
        "export function target() {}\n",
    )?;
    fs::write(
        directory.path().join("view.tsx"),
        "export const View = () => null;\n",
    )?;
    fs::write(
        directory.path().join("pkg/index.tsx"),
        "export const item = 1;\n",
    )?;
    let source = directory.path().join("main.js");
    fs::write(
        &source,
        r#"import { target } from "./target.js";
import { View } from "./view.jsx";
import { item } from "./pkg";
export function run() { target(); View(); return item; }
"#,
    )?;
    let mut engine = Engine::default();
    let extraction = engine.extract(&source)?;
    assert!(
        extraction
            .edges
            .iter()
            .any(|edge| matches!(edge.string("relation").as_str(), "imports" | "imports_from")),
        "edges={:?}",
        extraction.edges
    );
    Ok(())
}

#[test]
fn javascript_modules_reexports_require_and_decorators_keep_compass_contracts()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    fs::create_dir_all(directory.path().join("pkg"))?;
    fs::write(
        directory.path().join("target.ts"),
        "export function target() {}\nexport function second() {}\n",
    )?;
    fs::write(
        directory.path().join("pkg/index.ts"),
        "export const item = 1;\n",
    )?;
    let barrel = directory.path().join("barrel.ts");
    fs::write(
        &barrel,
        "export { target, second } from './target';\nexport * from './pkg';\n",
    )?;
    let decorated = directory.path().join("decorated.ts");
    fs::write(
        &decorated,
        "import { Injectable } from '@nestjs/common';\n@Injectable()\nexport class Service {}\n",
    )?;
    let common_js = directory.path().join("loader.cjs");
    fs::write(
        &common_js,
        "const { target } = require('./target');\nmodule.exports = { target };\n",
    )?;

    let mut engine = Engine::default();
    let barrel_facts = engine.extract(&barrel)?;
    assert_eq!(
        barrel_facts
            .edges
            .iter()
            .filter(|edge| {
                edge.string("relation") == "re_exports" && edge.string("context") == "export"
            })
            .count(),
        0,
        "file-level re-export edges belong to the collection resolver"
    );
    assert_eq!(
        barrel_facts
            .edges
            .iter()
            .filter(|edge| {
                edge.string("relation") == "imports_from" && edge.string("context") == "re-export"
            })
            .count(),
        2
    );
    assert!(barrel_facts.edges.iter().any(|edge| {
        edge.string("relation") == "re_exports" && edge.string("context") == "re-export"
    }));

    let decorated_facts = engine.extract(&decorated)?;
    let decorator_id = make_id(&["Injectable"]);
    assert!(decorated_facts.nodes.iter().any(|node| {
        node.id == decorator_id
            && node.label() == "Injectable"
            && node.string("source_file").is_empty()
    }));
    assert!(decorated_facts.edges.iter().any(|edge| {
        edge.target == decorator_id
            && edge.string("relation") == "references"
            && edge.string("context") == "decorator"
    }));
    assert!(decorated_facts.edges.iter().any(|edge| {
        edge.target == make_id(&["ref", "@nestjs/common"])
            && edge.string("relation") == "imports_from"
    }));
    assert!(
        !decorated_facts
            .edges
            .iter()
            .any(|edge| { edge.string("relation") == "imports" && edge.target == decorator_id })
    );

    let cjs_facts = engine.extract(&common_js)?;
    assert!(
        cjs_facts
            .edges
            .iter()
            .any(|edge| edge.string("relation") == "imports_from")
    );
    assert!(
        cjs_facts
            .edges
            .iter()
            .any(|edge| edge.string("relation") == "imports")
    );
    Ok(())
}

#[test]
fn typescript_type_star_reexports_do_not_discard_earlier_barrel_edges() -> Result<(), Box<dyn Error>>
{
    let directory = tempfile::tempdir()?;
    let value = directory.path().join("value.ts");
    let types = directory.path().join("types.ts");
    let barrel = directory.path().join("index.ts");
    fs::write(&value, "export const value = 1;\n")?;
    fs::write(&types, "export interface Value {}\n")?;
    fs::write(
        &barrel,
        "export * from './value.ts';\nexport type * from './types.ts';\n",
    )?;

    let mut engine = Engine::default();
    let extraction = engine.extract(&barrel)?;
    assert_ne!(
        extraction
            .extensions
            .get(EXTRACTION_QUALITY_EXTENSION)
            .and_then(serde_json::Value::as_str),
        Some(EXTRACTION_QUALITY_PARTIAL),
        "a supported type-only star re-export must not mark the whole file partial"
    );
    assert_eq!(
        extraction
            .edges
            .iter()
            .filter(|edge| {
                edge.string("relation") == "imports_from" && edge.string("context") == "re-export"
            })
            .count(),
        2,
        "both star re-exports must remain source-grounded"
    );
    Ok(())
}

#[test]
fn typescript_import_type_query_gap_does_not_quarantine_namespace_heritage()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("types.ts");
    fs::write(
        &path,
        r#"export namespace Docs {
  export interface Parent {}
  export interface Before extends Parent {}
  export type Item = (typeof import("./items.ts").items)[number];
  export interface After extends Parent {}
}
"#,
    )?;

    let mut engine = Engine::default();
    let extraction = engine.extract(&path)?;
    assert_ne!(
        extraction
            .extensions
            .get(EXTRACTION_QUALITY_EXTENSION)
            .and_then(serde_json::Value::as_str),
        Some(EXTRACTION_QUALITY_PARTIAL)
    );
    assert_eq!(
        extraction
            .edges
            .iter()
            .filter(|edge| edge.string("relation") == "inherits")
            .count(),
        2
    );
    Ok(())
}

#[test]
fn javascript_and_typescript_heritage_edges_keep_exact_base_sites() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let typescript = directory.path().join("types.ts");
    let javascript = directory.path().join("classes.js");
    fs::write(
        &typescript,
        "interface ContextOptions {}\ninterface AddOptions<T> extends ContextOptions {}\nclass Base {}\nclass Derived extends Base implements ContextOptions {}\n",
    )?;
    fs::write(
        &javascript,
        "class ErrorBase {}\nclass DomainError extends ErrorBase {}\n",
    )?;

    let mut engine = Engine::default();
    let typescript_facts = engine.extract(&typescript)?;
    let type_id = |label: &str| {
        typescript_facts
            .nodes
            .iter()
            .find(|node| node.label() == label)
            .map(|node| node.id.as_str())
    };
    assert!(typescript_facts.edges.iter().any(|edge| {
        Some(edge.source.as_str()) == type_id("AddOptions")
            && Some(edge.target.as_str()) == type_id("ContextOptions")
            && edge.string("relation") == "inherits"
            && edge.string("source_location") == "L2"
    }));
    assert!(typescript_facts.edges.iter().any(|edge| {
        Some(edge.source.as_str()) == type_id("Derived")
            && Some(edge.target.as_str()) == type_id("Base")
            && edge.string("relation") == "inherits"
            && edge.string("source_location") == "L4"
    }));
    assert!(typescript_facts.edges.iter().any(|edge| {
        Some(edge.source.as_str()) == type_id("Derived")
            && Some(edge.target.as_str()) == type_id("ContextOptions")
            && edge.string("relation") == "implements"
            && edge.string("source_location") == "L4"
    }));

    let javascript_facts = engine.extract(&javascript)?;
    let javascript_id = |label: &str| {
        javascript_facts
            .nodes
            .iter()
            .find(|node| node.label() == label)
            .map(|node| node.id.as_str())
    };
    assert!(javascript_facts.edges.iter().any(|edge| {
        Some(edge.source.as_str()) == javascript_id("DomainError")
            && Some(edge.target.as_str()) == javascript_id("ErrorBase")
            && edge.string("relation") == "inherits"
            && edge.string("source_location") == "L2"
    }));
    Ok(())
}

#[test]
fn javascript_module_object_bindings_have_explicit_variable_kind() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("app.ts");
    fs::write(
        &path,
        r#"
import express from "express";
const app = express();
const settings = { enabled: true };
const { EventEmitter, PassThrough: Stream } = require("events");
const [nodeMajorVersion] = process.versions.node.split(".");
app.get("/health", health);
function health() { return "ok"; }
"#,
    )?;

    let extraction = Engine::default().extract(&path)?;
    for name in [
        "app",
        "settings",
        "EventEmitter",
        "Stream",
        "nodeMajorVersion",
    ] {
        let binding = extraction
            .nodes
            .iter()
            .find(|node| node.label() == name)
            .ok_or_else(|| format!("missing {name} binding"))?;
        assert_eq!(
            binding.string("symbol_kind"),
            "variable",
            "binding={binding:#?}"
        );
    }
    assert!(!extraction.nodes.iter().any(|node| {
        matches!(
            node.label(),
            "EventEmitterPassThroughStream" | "nodeMajorVersionprocessversionsnodesplit"
        )
    }));
    Ok(())
}

#[test]
fn javascript_callback_values_are_references_without_invocation_evidence()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("callbacks.ts");
    fs::write(
        &path,
        r#"
function register(_callback) {}
function callback() {}
const handlers = [callback];
register(callback);
"#,
    )?;

    let extraction = Engine::default().extract(&path)?;
    let stem = path.with_extension("").to_string_lossy().replace('\\', "/");
    let callback_id = make_id(&[&stem, "callback"]);
    let callback_edges = extraction
        .edges
        .iter()
        .filter(|edge| edge.target == callback_id)
        .collect::<Vec<_>>();
    assert!(
        callback_edges.iter().any(|edge| {
            edge.string("relation") == "references" && edge.string("context") == "argument"
        }),
        "callback id={callback_id}; callback edges={callback_edges:?}"
    );
    assert!(
        callback_edges.iter().any(|edge| {
            edge.string("relation") == "references" && edge.string("context") == "collection"
        }),
        "callback edges={callback_edges:?}"
    );
    assert!(
        callback_edges
            .iter()
            .all(|edge| edge.string("relation") != "indirect_call")
    );
    Ok(())
}

#[test]
fn javascript_typescript_import_kinds_commonjs_exports_and_jsx_references()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let target = directory.path().join("target.ts");
    let consumer = directory.path().join("consumer.tsx");
    let common_js = directory.path().join("loader.cjs");
    fs::write(
        &target,
        "export default function render() {}\nexport const value = 1;\nexport class Button {}\n",
    )?;
    fs::write(
        &consumer,
        r#"import render, * as UI from "./target.js";
import { Button } from "./target.js";
import type { Button as ButtonType } from "./target.js";
export function App(value: ButtonType) {
  return <UI.Button value={value} onClick={render} />;
}
const local = Button;
"#,
    )?;
    fs::write(
        &common_js,
        "function handler() {}\nmodule.exports = { handler };\nexports.named = handler;\n",
    )?;

    let mut engine = Engine::default();
    let consumer_facts = engine.extract(&consumer)?;
    let import_edges = consumer_facts
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "imports")
        .collect::<Vec<_>>();
    assert!(import_edges.iter().any(|edge| {
        edge.string("imported_name") == "default"
            && edge.string("import_kind") == "default"
            && edge.string("local_name") == "render"
    }));
    assert!(import_edges.iter().any(|edge| {
        edge.string("imported_name") == "*"
            && edge.string("import_kind") == "namespace"
            && edge.string("local_name") == "UI"
    }));
    assert!(import_edges.iter().any(|edge| {
        edge.string("imported_name") == "Button"
            && edge
                .attributes
                .get("type_only")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
            && edge.string("local_name") == "ButtonType"
    }));
    assert!(
        consumer_facts.edges.iter().any(|edge| {
            edge.string("relation") == "references" && edge.string("context") == "jsx"
        }),
        "edges={:#?}",
        consumer_facts.edges
    );

    let common_js_facts = engine.extract(&common_js)?;
    let handler = make_id(&[&common_js.with_extension("").to_string_lossy(), "handler"]);
    let commonjs_exports = common_js_facts
        .edges
        .iter()
        .filter(|edge| {
            edge.string("relation") == "exports"
                && edge.target == handler
                && edge.string("module_format") == "commonjs"
        })
        .collect::<Vec<_>>();
    assert!(
        commonjs_exports
            .iter()
            .any(|edge| edge.string("export_name") == "handler")
    );
    assert!(
        commonjs_exports
            .iter()
            .any(|edge| edge.string("export_name") == "named")
    );
    Ok(())
}

#[test]
fn dart_exports_have_explicit_resource_targets() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("exports.dart");
    fs::write(&path, "export 'rich.dart';\n")?;

    let extraction = Engine::default().extract(&path)?;
    let export = extraction
        .edges
        .iter()
        .find(|edge| edge.string("relation") == "exports")
        .ok_or("missing Dart export")?;
    let target = extraction
        .nodes
        .iter()
        .find(|node| node.id == export.target)
        .ok_or("missing Dart export target")?;

    assert_eq!(target.string("symbol_kind"), "resource");
    assert_eq!(
        export.string("target_file"),
        directory.path().join("rich.dart").to_string_lossy()
    );
    Ok(())
}

#[test]
fn repeated_anonymous_class_methods_receive_cross_definition_discriminators()
-> Result<(), Box<dyn Error>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../compass-cli/assets/vendor/pierre-diffs-v1.2.12.js");
    let source = fs::read(&path)?;

    let extraction = Engine::default().extract_source(&path, &source)?;
    let methods = extraction
        .nodes
        .iter()
        .filter(|node| node.label() == "cleanUp()")
        .collect::<Vec<_>>();
    assert!(methods.len() >= 2, "methods={methods:?}");
    assert_eq!(
        methods
            .iter()
            .map(|node| node.string("overload_discriminator"))
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        methods.len(),
        "methods={methods:?}"
    );
    Ok(())
}

#[test]
fn go_grouped_and_method_predeclared_types_keep_definition_anchors() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let package = directory.path().join("generated");
    fs::create_dir(&package)?;
    let path = package.join("types.go");
    let source = br#"package generated

func (value *Later) Use() {}

type Later struct {
    Value string
}

type (
    optionFunc[C any] func(*C)
)
"#;
    let extraction = Engine::default().extract_source(&path, source)?;
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing Go semantic evidence")?;
    let later = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "generated.Later")
        .ok_or("missing later type")?;
    assert_eq!(later.name, "Later");
    assert_eq!(later.range.start_line, 5);
    assert_eq!(later.kind, "struct");

    let option = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "generated.optionFunc")
        .ok_or("missing grouped generic type")?;
    assert_eq!(option.name, "optionFunc");
    assert_eq!(option.range.start_line, 10);
    assert_eq!(option.kind, "type_alias");
    assert_eq!(
        evidence
            .declarations
            .iter()
            .filter(|declaration| declaration.id == later.id || declaration.id == option.id)
            .count(),
        2
    );
    Ok(())
}

#[test]
fn objective_c_go_and_swift_fixtures_cover_type_members_calls_and_imports()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    fs::write(
        directory.path().join("Local.h"),
        "@interface Local : NSObject\n@end\n",
    )?;
    let fixtures = [
        (
            "Service.m",
            r#"NS_ASSUME_NONNULL_BEGIN
#import <Foundation/Foundation.h>
#import "Local.h"
@import UIKit;

@protocol Child <NSObject>
- (void)required;
@end

@interface Service : NSObject <Child>
@property(nonatomic, strong) ExternalType *field;
- (void)helper;
- (Result *)run:(Input *)input;
@end

@implementation Service
- (void)helper {}
- (Result *)run:(Input *)input {
    ExternalType *local = [[ExternalType alloc] init];
    [self helper];
    [ExternalType alloc];
    self.helper;
    @selector(helper);
    return [local execute:input];
}
@end
NS_ASSUME_NONNULL_END
"#,
        ),
        (
            "service.go",
            r#"package service

import (
    "context"
    alias "example.com/project/dependency"
)

type Embedded interface { Base; Run(context.Context) error }
type Box[T any] struct { Value T; Client *alias.Client }

func NewBox[T any](value T) *Box[T] { return &Box[T]{Value: value} }
func (b *Box[T]) Run(ctx context.Context) error {
    defer cleanup()
    go notify(b.Value)
    alias.Handle(ctx)
    return b.Client.Execute(ctx)
}
"#,
        ),
        (
            "Service.swift",
            r#"import Foundation

protocol Runnable: AnyObject { func run(_ input: Input) async throws -> Output }
class Base {}
final class Service<T: Contract>: Base, Runnable {
    let dependency: Dependency
    init(dependency: Dependency) { self.dependency = dependency }
    func run(_ input: Input) async throws -> Output {
        let value: Intermediate = try await dependency.load(input)
        return helper(value)
    }
}
extension Service { func helper(_ value: Intermediate) -> Output { Output(value) } }
enum Mode { case fast; case safe }
struct Wrapper { var service: Service<Concrete> }
"#,
        ),
    ];

    let mut engine = Engine::default();
    for (name, text) in fixtures {
        let path = directory.path().join(name);
        fs::write(&path, text)?;
        let extraction = engine.extract(&path)?;
        if let Some(evidence) = extraction.semantic_evidence.as_ref() {
            assert!(
                evidence.declarations.len() >= 3,
                "{name}: {:?}",
                evidence.declarations
            );
            assert!(
                !evidence.bindings.is_empty(),
                "{name}: {:?}",
                evidence.bindings
            );
            assert!(
                evidence.candidates.iter().any(|candidate| matches!(
                    candidate.relation,
                    CandidateRelation::Calls
                        | CandidateRelation::References
                        | CandidateRelation::Embeds
                        | CandidateRelation::Implements
                )),
                "{name}: {:?}",
                evidence.candidates
            );
            continue;
        }
        assert!(
            extraction.nodes.len() >= 3,
            "{name}: {:?}",
            extraction.nodes
        );
        assert!(
            extraction.edges.iter().any(|edge| {
                matches!(edge.string("relation").as_str(), "imports" | "imports_from")
            }),
            "{name}: {:?}",
            extraction.edges
        );
        assert!(
            extraction.edges.iter().any(|edge| {
                matches!(
                    edge.string("relation").as_str(),
                    "calls" | "references" | "inherits" | "implements"
                )
            }),
            "{name}: {:?}",
            extraction.edges
        );
    }
    let objc = engine.extract(&directory.path().join("Service.m"))?;
    assert!(objc.extensions.contains_key("objc_type_table"));
    assert!(!objc.raw_calls.as_deref().unwrap_or_default().is_empty());
    Ok(())
}

#[test]
fn cpp_and_dream_maker_fixtures_cover_qualified_generics_overrides_and_receivers()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    fs::write(directory.path().join("local.hpp"), "struct Local {};\n")?;
    fs::write(
        directory.path().join("helpers.dm"),
        "/proc/helper()\n\treturn 1\n",
    )?;
    let fixtures = [
        (
            "advanced.cpp",
            r#"#include "local.hpp"
#include <vector>
namespace api { class Base {}; void global_call(); }
template <typename T> class GenericBase {};
class Service : public api::Base, public GenericBase<Local> {
public:
    Local value;
    std::vector<Local*> items;
    Local *pointer, &reference;
    Local* run(const Local& input) {
        this->helper();
        api::global_call();
        pointer->execute();
        return factory(input);
    }
    void helper() {}
};
Local* free_call(Service& service) { service.helper(); return create(); }
"#,
        ),
        (
            "advanced.dm",
            r#"#include "helpers.dm"
/proc/log_event(message)
	world.log << message

/datum/base
	proc/run()
		return helper()

/datum/service
	parent_type = /datum/base
	var/datum/base/dependency
	proc/helper()
		return 1
	proc/run()
		var/datum/service/local = new /datum/service()
		local.helper()
		src.helper()
		return ..()

/datum/service/proc/external()
	log_event("external")
	return new /datum/base()
"#,
        ),
    ];
    let mut engine = Engine::default();
    for (name, source) in fixtures {
        let path = directory.path().join(name);
        fs::write(&path, source)?;
        let extraction = engine.extract(&path)?;
        assert!(
            extraction.nodes.len() >= 4,
            "{name}: {:?}",
            extraction.nodes
        );
        assert!(
            extraction
                .edges
                .iter()
                .any(|edge| edge.string("relation") == "imports"
                    || edge.string("relation") == "imports_from")
        );
        assert!(
            extraction.edges.iter().any(|edge| {
                matches!(
                    edge.string("relation").as_str(),
                    "calls" | "references" | "inherits"
                )
            }),
            "{name}: {:?}",
            extraction.edges
        );
        if name == "advanced.cpp" {
            assert!(
                !extraction
                    .raw_calls
                    .as_deref()
                    .unwrap_or_default()
                    .is_empty()
            );
        }
    }
    Ok(())
}

#[test]
fn dotnet_pascal_xaml_and_template_fixtures_cover_project_and_ui_relationships()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let fixtures = [
        (
            "Service.cs",
            r#"using System;
using System.Collections.Generic;
namespace Compass.App;
public interface IRunner<in T> { Result Run(T input); }
public abstract class Base<T> where T : Contract { protected T Value { get; init; } }
public sealed record Service<T>(Dependency Dependency) : Base<T>, IRunner<Input>
    where T : Contract, new()
{
    public event EventHandler? Changed;
    public Result Run(Input input)
    {
        var item = Dependency.Load(input);
        Changed?.Invoke(this, EventArgs.Empty);
        return Helper.Create<Result>(item);
    }
}
public enum Mode { Fast, Safe }
public delegate Output Transform(Input input);
"#,
        ),
        (
            "Compass.csproj",
            r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup><TargetFramework>net9.0</TargetFramework></PropertyGroup>
  <ItemGroup>
    <ProjectReference Include="../Core/Core.csproj" />
    <PackageReference Include="System.Text.Json" Version="9.0.0" />
    <Compile Include="Generated.cs" Link="Shared/Generated.cs" />
  </ItemGroup>
</Project>"#,
        ),
        (
            "MainWindow.xaml",
            r#"<Window x:Class="Compass.App.MainWindow"
 xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
 xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml"
 xmlns:local="clr-namespace:Compass.App">
 <Grid DataContext="{Binding Main}">
  <local:GraphView x:Name="Graph" ItemsSource="{Binding Nodes}" />
  <Button Click="OnRefresh" Command="{Binding RefreshCommand}" />
 </Grid>
</Window>"#,
        ),
        (
            "units.pas",
            r#"unit Units;
interface
uses SysUtils, Classes;
type
  IRunner = interface
    function Run(const Input: TInput): TOutput;
  end;
  TService = class(TBase, IRunner)
  private
    FDependency: TDependency;
    procedure Helper(Sender: TObject);
  public
    constructor Create(const Dependency: TDependency);
    function Run(const Input: TInput): TOutput; override;
    property Dependency: TDependency read FDependency write FDependency;
  end;
implementation
constructor TService.Create(const Dependency: TDependency);
begin inherited Create; FDependency := Dependency; end;
procedure TService.Helper(Sender: TObject);
begin FDependency.Notify(Sender); end;
function TService.Run(const Input: TInput): TOutput;
begin Result := FDependency.Execute(Input); Helper(Self); end;
end."#,
        ),
        (
            "MainForm.dfm",
            r#"object MainForm: TMainForm
  Caption = 'Compass'
  object RefreshButton: TButton
    OnClick = RefreshButtonClick
  end
  object DataSource1: TDataSource
    DataSet = Query1
  end
end"#,
        ),
        (
            "Component.vue",
            r#"<script setup lang="ts">
import { computed, ref } from 'vue'
import GraphView from './GraphView.vue'
const props = defineProps<{ nodes: Node[] }>()
const emit = defineEmits<{ select: [Node] }>()
const count = computed(() => props.nodes.length)
function choose(node: Node) { emit('select', node) }
</script>
<template><GraphView v-for="node in props.nodes" :key="node.id" @click="choose(node)" /></template>"#,
        ),
        (
            "Widget.svelte",
            r#"<script lang="ts">
 import { onMount } from 'svelte';
 export let service: Service;
 $: result = service.compute();
 onMount(() => service.start());
</script>
{#each result as item}<button on:click={() => service.select(item)}>{item.name}</button>{/each}"#,
        ),
        (
            "Card.astro",
            r#"---
import Layout from './Layout.astro';
const { title, items } = Astro.props;
const rows = items.map(formatRow);
---
<Layout title={title}>{rows.map(row => <span>{row}</span>)}</Layout>"#,
        ),
    ];

    let mut engine = Engine::default();
    for (name, source) in fixtures {
        let path = directory.path().join(name);
        fs::write(&path, source)?;
        let extraction = engine.extract(&path)?;
        assert!(!extraction.nodes.is_empty(), "{name}");
        assert!(
            extraction.edges.iter().any(|edge| matches!(
                edge.string("relation").as_str(),
                "contains"
                    | "calls"
                    | "references"
                    | "imports"
                    | "imports_from"
                    | "inherits"
                    | "implements"
            )),
            "{name}: {:?}",
            extraction.edges
        );
    }
    Ok(())
}
