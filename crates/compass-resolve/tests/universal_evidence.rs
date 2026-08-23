use std::collections::HashMap;
use std::error::Error;
use std::path::Path;

use compass_graph::{BuildEvidence, InferenceLevel, apply_inference_level, normalize_v1};
use compass_languages::{CandidateRelation, Engine, EvidenceLimits, validate_evidence};
use compass_model::code_graph::NodeKind;
use compass_resolve::evidence::{
    ResolutionDecision, UniversalResolutionIndex, UniversalResolutionLimits,
};
use compass_resolve::{resolve, resolve_with_root};

#[test]
fn rust_method_navigation_uses_definition_extent_without_weakening_exact_evidence()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("src/lib.rs");
    std::fs::create_dir_all(path.parent().ok_or("missing source parent")?)?;
    let source = b"struct Service;\nimpl Service {\n    fn run(&self) {\n        let _value = 1;\n    }\n}\n";
    std::fs::write(&path, source)?;

    let extracted = Engine::default().extract(&path)?;
    let resolved = resolve_with_root(
        &[extracted],
        &HashMap::from([(
            path.to_string_lossy().into_owned(),
            String::from_utf8(source.to_vec())?,
        )]),
        directory.path(),
    );
    let build = BuildEvidence::from_extraction(
        directory.path(),
        &resolved,
        "sha256:rust-navigation-range",
    )?;
    let graph = normalize_v1(resolved, build)?;
    let method = graph
        .nodes
        .iter()
        .find(|node| node.kind == NodeKind::Method && node.name == ".run()")
        .ok_or("missing Rust method")?;

    let navigation = method.source.as_ref().ok_or("missing method source")?;
    assert_eq!(navigation.start_line, 3);
    assert_eq!(navigation.start_column, 4);
    assert_eq!(navigation.end_line, 5);
    assert_eq!(navigation.end_column, 5);

    let identifier = method
        .evidence
        .first()
        .and_then(|evidence| evidence.anchors.first())
        .ok_or("missing exact method evidence")?;
    assert_eq!(identifier.start_line, 3);
    assert_eq!(identifier.start_column, 7);
    assert_eq!(identifier.end_line, 3);
    assert_eq!(identifier.end_column, 10);
    assert_eq!(
        &source[usize::try_from(identifier.start_byte)?..usize::try_from(identifier.end_byte)?],
        b"run"
    );
    Ok(())
}

#[test]
fn swift_protocol_conformance_publishes_implements() -> Result<(), Box<dyn Error>> {
    let source = br#"protocol Store {}
struct UserStore: Store {}
class Box: Store {}
"#;
    let source_file = "Sources/Store.swift";
    let extracted = Engine::default()
        .extract_source_combined(Path::new(source_file), source_file, source)?
        .graph;
    let evidence = extracted
        .semantic_evidence
        .as_ref()
        .ok_or("missing Swift universal evidence")?;
    validate_evidence(evidence, EvidenceLimits::default())?;
    let resolved = resolve(
        &[extracted],
        &HashMap::from([(source_file.to_owned(), String::from_utf8(source.to_vec())?)]),
    );
    assert!(resolved.error.is_none(), "{:#?}", resolved.error);
    let node = |qualified_name: &str| {
        resolved
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == qualified_name)
    };
    let store = node("Store").ok_or("missing Store")?;
    for implementer in ["UserStore", "Box"] {
        let source_node = node(implementer).ok_or("missing Swift implementer")?;
        assert!(
            resolved.edges.iter().any(|edge| {
                edge.source == source_node.id
                    && edge.target == store.id
                    && edge.string("relation") == "implements"
            }),
            "missing Swift implements edge for {implementer}"
        );
    }
    Ok(())
}

#[test]
fn dart_part_receiver_dispatch_resolves_repeated_local_calls() -> Result<(), Box<dyn Error>> {
    let library = br#"library wave;
abstract class Store { void save(String value); }
class UserStore implements Store {
  void save(String value) {}
}
"#;
    let part = br#"part of wave;
void repeated(UserStore store) { store.save('a'); store.save('b'); }
void dynamicCall(dynamic receiver) { receiver.unknown(); }
"#;
    let library_file = "lib/library.dart";
    let part_file = "lib/src/part.dart";
    let library_extracted = Engine::default()
        .extract_source_combined(Path::new(library_file), library_file, library)?
        .graph;
    let part_extracted = Engine::default()
        .extract_source_combined(Path::new(part_file), part_file, part)?
        .graph;
    let resolved = resolve(
        &[library_extracted, part_extracted],
        &HashMap::from([
            (
                library_file.to_owned(),
                String::from_utf8(library.to_vec())?,
            ),
            (part_file.to_owned(), String::from_utf8(part.to_vec())?),
        ]),
    );
    assert!(resolved.error.is_none(), "{:#?}", resolved.error);
    let node = |qualified_name: &str| {
        resolved
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == qualified_name)
    };
    let save = node("wave.UserStore.save").ok_or("missing UserStore.save")?;
    let repeated = node("wave.repeated").ok_or("missing repeated")?;
    assert_eq!(
        resolved
            .edges
            .iter()
            .filter(|edge| {
                edge.source == repeated.id
                    && edge.target == save.id
                    && edge.string("relation") == "calls"
            })
            .count(),
        2,
        "missing repeated Dart receiver dispatch edges"
    );
    Ok(())
}

#[test]
fn dart_source_supplement_recovers_local_calls_and_constructions() -> Result<(), Box<dyn Error>> {
    let source = br#"class Store {
  Store();
  void save() {}
  void run() {
    save();
    final callback = () => save();
    Store();
    unknown();
    // save();
    final text = 'save()';
  }
}
class _Hidden {
  _Hidden();
}
void build() { _Hidden(); }
class _Implicit {}
void buildImplicit() { _Implicit(); }
"#;
    let source_file = "lib/store.dart";
    let extracted = Engine::default()
        .extract_source_combined(Path::new(source_file), source_file, source)?
        .graph;
    let resolved = resolve(
        &[extracted],
        &HashMap::from([(source_file.to_owned(), String::from_utf8(source.to_vec())?)]),
    );
    assert!(resolved.error.is_none(), "{:#?}", resolved.error);
    let node = |qualified_name: &str| {
        resolved
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == qualified_name)
    };
    let run = node("Store.run").ok_or("missing Store.run")?;
    let save = node("Store.save").ok_or("missing Store.save")?;
    let constructor = node("Store.Store").ok_or("missing Store constructor")?;
    let hidden_constructor = node("_Hidden._Hidden").ok_or("missing private constructor")?;
    let build = node("build").ok_or("missing build function")?;
    let implicit_class = node("_Implicit").ok_or("missing implicit constructor class")?;
    let build_implicit = node("buildImplicit").ok_or("missing implicit constructor caller")?;
    assert_eq!(
        resolved
            .edges
            .iter()
            .filter(|edge| {
                edge.source == run.id
                    && edge.target == save.id
                    && edge.string("relation") == "calls"
            })
            .count(),
        2,
        "local Dart calls should be recovered once per source occurrence"
    );
    assert_eq!(
        resolved
            .edges
            .iter()
            .filter(|edge| {
                edge.source == run.id
                    && edge.target == constructor.id
                    && edge.string("relation") == "calls"
            })
            .count(),
        1,
        "local Dart construction should resolve to the constructor"
    );
    assert_eq!(
        resolved
            .edges
            .iter()
            .filter(|edge| {
                edge.source == build.id
                    && edge.target == hidden_constructor.id
                    && edge.string("relation") == "calls"
            })
            .count(),
        1,
        "private Dart constructions should resolve to their constructor"
    );
    assert_eq!(
        resolved
            .edges
            .iter()
            .filter(|edge| {
                edge.source == build_implicit.id
                    && edge.target == implicit_class.id
                    && matches!(edge.string("relation").as_str(), "calls" | "instantiates")
            })
            .count(),
        1,
        "implicit Dart constructions should resolve to their class"
    );
    assert!(
        !resolved.edges.iter().any(|edge| {
            edge.source == run.id
                && edge.string("relation") == "calls"
                && edge.string("label") == "unknown"
        }),
        "unresolved Dart calls must remain fail-closed"
    );
    Ok(())
}

#[test]
fn groovy_local_implements_publishes_direct_base_edge() -> Result<(), Box<dyn Error>> {
    let source = br#"package wave
interface Store {}
class UserStore implements Store {
    void save() {}
}
"#;
    let source_file = "src/Store.groovy";
    let extracted = Engine::default()
        .extract_source_combined(Path::new(source_file), source_file, source)?
        .graph;
    let evidence = extracted
        .semantic_evidence
        .as_ref()
        .ok_or("missing Groovy universal evidence")?;
    validate_evidence(evidence, EvidenceLimits::default())?;
    let resolved = resolve(
        &[extracted],
        &HashMap::from([(source_file.to_owned(), String::from_utf8(source.to_vec())?)]),
    );
    assert!(resolved.error.is_none(), "{:#?}", resolved.error);
    let node = |qualified_name: &str| {
        resolved
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == qualified_name)
    };
    let store = node("wave.Store").ok_or("missing Store")?;
    let implementer = node("wave.UserStore").ok_or("missing UserStore")?;
    assert!(resolved.edges.iter().any(|edge| {
        edge.source == implementer.id
            && edge.target == store.id
            && edge.string("relation") == "implements"
    }));
    Ok(())
}

#[test]
fn groovy_source_calls_exclude_declaration_headers() -> Result<(), Box<dyn Error>> {
    let source = br#"package wave
interface Store { void save(String value) }
class UserStore implements Store {
    String value
    UserStore() {}
    void save(String value) { this.value = value }
    void route() { save('users') }
}
class Specification extends spock.lang.Specification {
    def "stores users"() { expect: new UserStore().save('ok') }
}
"#;
    let source_file = "Module.groovy";
    let extracted = Engine::default()
        .extract_source_combined(Path::new(source_file), source_file, source)?
        .graph;
    let resolved = resolve(
        &[extracted],
        &HashMap::from([(source_file.to_owned(), String::from_utf8(source.to_vec())?)]),
    );
    assert!(resolved.error.is_none(), "{:#?}", resolved.error);
    let mut call_sources = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls")
        .filter_map(|edge| {
            resolved
                .nodes
                .iter()
                .find(|node| node.id == edge.source)
                .map(|node| node.string("qualified_name"))
        })
        .collect::<Vec<_>>();
    call_sources.sort_unstable();
    assert_eq!(
        call_sources,
        vec![
            "wave.Specification.stores users",
            "wave.Specification.stores users",
            "wave.UserStore.route"
        ]
    );
    let feature = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "wave.Specification.stores users")
        .ok_or("missing Spock feature")?;
    let constructor = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "wave.UserStore.UserStore")
        .ok_or("missing UserStore constructor")?;
    assert!(resolved.edges.iter().any(|edge| {
        edge.source == feature.id
            && edge.target == constructor.id
            && edge.string("relation") == "calls"
    }));
    Ok(())
}

#[test]
fn scala_local_receiver_dispatch_resolves_inherited_and_extension_calls()
-> Result<(), Box<dyn Error>> {
    let source = br#"package wave
trait Store { def save(value: String): Unit }
final class UserStore extends Store {
  override def save(value: String): Unit = ()
  def route(): Unit = save("users")
}
extension (store: UserStore) def repeated(): Unit = { store.save("a"); store.save("b") }
object UserStore { def apply(): UserStore = new UserStore() }
"#;
    let source_file = "src/Module.scala";
    let extracted = Engine::default()
        .extract_source_combined(Path::new(source_file), source_file, source)?
        .graph;
    let resolved = resolve(
        &[extracted],
        &HashMap::from([(source_file.to_owned(), String::from_utf8(source.to_vec())?)]),
    );
    assert!(resolved.error.is_none(), "{:#?}", resolved.error);
    let node = |qualified_name: &str| {
        resolved
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == qualified_name)
    };
    let save = node("wave.UserStore.save").ok_or("missing UserStore.save")?;
    let route = node("wave.UserStore.route").ok_or("missing UserStore.route")?;
    let repeated = node("wave.repeated").ok_or("missing repeated extension")?;
    let apply = node("wave.UserStore.apply").ok_or("missing UserStore.apply")?;
    let user_store = node("wave.UserStore").ok_or("missing UserStore class")?;
    assert!(resolved.edges.iter().any(|edge| {
        edge.source == route.id && edge.target == save.id && edge.string("relation") == "calls"
    }));
    assert!(
        resolved
            .edges
            .iter()
            .filter(|edge| {
                edge.source == repeated.id
                    && edge.target == save.id
                    && edge.string("relation") == "calls"
            })
            .count()
            >= 2,
        "missing repeated extension dispatch edges"
    );
    assert!(
        resolved.edges.iter().any(|edge| {
            edge.source == apply.id
                && edge.target == user_store.id
                && matches!(edge.string("relation").as_str(), "calls" | "instantiates")
        }),
        "missing Scala companion construction edge"
    );
    Ok(())
}

#[test]
fn ambiguous_owned_scopes_fall_back_to_the_exact_declaration_anchor() -> Result<(), Box<dyn Error>>
{
    let source = b"struct Service;\nimpl Service {\n    fn run(&self) {\n        let _value = 1;\n    }\n}\n";
    let source_file = "src/lib.rs";
    let mut extracted = Engine::default().extract_source(Path::new(source_file), source)?;
    let evidence = extracted
        .semantic_evidence
        .as_mut()
        .ok_or("missing Rust semantic evidence")?;
    let declaration_id = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.kind == "method" && declaration.name == "run")
        .map(|declaration| declaration.id.clone())
        .ok_or("missing run declaration")?;
    let mut duplicate = evidence
        .scopes
        .iter()
        .find(|scope| scope.owner_declaration_id.as_deref() == Some(declaration_id.as_str()))
        .cloned()
        .ok_or("missing run scope")?;
    duplicate.id = "scope:duplicate-run".to_owned();
    evidence.scopes.push(duplicate);

    let resolved = resolve(
        &[extracted],
        &HashMap::from([(source_file.to_owned(), String::from_utf8(source.to_vec())?)]),
    );
    let method = resolved
        .nodes
        .iter()
        .find(|node| node.string("symbol_kind") == "method" && node.string("label") == ".run()")
        .ok_or("missing resolved Rust method")?;
    assert!(!method.attributes.contains_key("source_anchor"));
    assert_eq!(
        method
            .attributes
            .get("line_start")
            .and_then(serde_json::Value::as_u64),
        Some(3)
    );
    assert_eq!(
        method
            .attributes
            .get("line_end")
            .and_then(serde_json::Value::as_u64),
        Some(3)
    );
    Ok(())
}

#[test]
fn bounded_universal_indexes_preserve_ambiguous_declarations() -> Result<(), Box<dyn Error>> {
    let source =
        b"def target():\n    pass\n\ndef target():\n    pass\n\ndef caller():\n    target()\n";
    let extraction = Engine::default().extract_source(Path::new("module.py"), source)?;
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing Python semantic evidence")?;
    let candidate = evidence
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == CandidateRelation::Calls && candidate.target_spelling == "target"
        })
        .ok_or("missing target call candidate")?;

    let index = UniversalResolutionIndex::new(
        std::slice::from_ref(evidence),
        UniversalResolutionLimits {
            candidates_per_lookup: 1,
            ..UniversalResolutionLimits::default()
        },
    )?;
    assert!(matches!(
        index.resolve(&candidate.id),
        ResolutionDecision::Ambiguous { candidate_count: 2 }
    ));
    Ok(())
}

#[test]
fn aggregate_universal_limits_are_checked_before_reserving_indexes() -> Result<(), Box<dyn Error>> {
    let mut engine = Engine::default();
    let left = engine.extract_source(Path::new("left.py"), b"def left():\n    pass\n")?;
    let right = engine.extract_source(Path::new("right.py"), b"def right():\n    pass\n")?;
    let batches = [
        left.semantic_evidence
            .ok_or("missing left semantic evidence")?,
        right
            .semantic_evidence
            .ok_or("missing right semantic evidence")?,
    ];

    let error = match UniversalResolutionIndex::new(
        &batches,
        UniversalResolutionLimits {
            declarations: 1,
            ..UniversalResolutionLimits::default()
        },
    ) {
        Ok(_) => return Err("aggregate declaration limit was not enforced".into()),
        Err(error) => error,
    };
    assert!(
        error.contains("aggregate declarations count"),
        "error={error}"
    );
    Ok(())
}

#[test]
fn markdown_documents_edges_resolve_to_universal_file_inventory() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let markdown_path = directory.path().join("guide.md");
    let rust_path = directory.path().join("documented.rs");
    std::fs::write(&markdown_path, "# Guide\n[Implementation](documented.rs)\n")?;
    std::fs::write(&rust_path, "pub fn documented() {}\n")?;

    let mut engine = Engine::default();
    let markdown = engine.extract(&markdown_path)?;
    let rust = engine.extract(&rust_path)?;
    let merged = resolve_with_root(&[markdown, rust], &HashMap::new(), directory.path());

    let edge = merged
        .edges
        .iter()
        .find(|edge| edge.string("relation") == "documents")
        .ok_or("missing documents edge")?;
    let target = merged
        .nodes
        .iter()
        .find(|node| node.id == edge.target)
        .ok_or("missing documents target")?;
    assert_eq!(target.string("symbol_kind"), "file");
    assert_eq!(target.string("source_file"), "documented.rs");

    let graph = compass_graph::build(&[merged], true, true, Some(directory.path()))?;
    assert!(graph.links.iter().any(|edge| {
        edge.attributes
            .get("relation")
            .and_then(serde_json::Value::as_str)
            == Some("documents")
    }));
    let inventory = [(&markdown_path, "markdown"), (&rust_path, "rust")]
        .into_iter()
        .map(|(path, language)| compass_graph::InventoryEvidence {
            path: path.clone(),
            language: Some(language.to_owned()),
            producer: format!("compass.languages.{language}"),
            status: compass_model::code_graph::ExtractionStatus::Extracted,
            reason: None,
        })
        .collect();
    let published = compass_graph::normalize_document_v1_with_inventory_best_effort(
        &graph,
        directory.path(),
        "test",
        None,
        inventory,
    )?;
    assert!(
        published
            .document
            .links
            .iter()
            .any(|edge| edge.kind.as_str() == "documents"),
        "published={:#?}",
        published.document
    );
    Ok(())
}

#[test]
fn markdown_project_links_resolve_exact_files_fragments_indexes_and_unique_wiki_stems()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let docs = directory.path().join("docs");
    let topic = docs.join("topic");
    std::fs::create_dir_all(&topic)?;
    let guide_path = docs.join("guide.md");
    let reference_path = docs.join("reference.md");
    let index_path = topic.join("README.md");
    let ambiguous_markdown_path = docs.join("dual.md");
    let ambiguous_mdx_path = docs.join("dual.mdx");
    let skill_path = docs.join("agent.skill");
    let code_paths = [
        (directory.path().join("src/module.py"), "python"),
        (directory.path().join("src/lib.rs"), "rust"),
        (directory.path().join("src/main.go"), "go"),
        (directory.path().join("src/Main.java"), "java"),
        (directory.path().join("src/index.ts"), "typescript"),
    ];
    std::fs::create_dir_all(code_paths[0].0.parent().ok_or("missing code parent")?)?;
    std::fs::write(
        &guide_path,
        concat!(
            "# Guide\n",
            "[exact](reference.md#target-section)\n",
            "[extensionless](reference#target-section)\n",
            "[root](/docs/reference.md#target-section)\n",
            "[encoded](reference.md#target%2Dsection)\n",
            "[[Reference#target-section]]\n",
            "[directory](topic/#overview)\n",
            "[skill](agent#rules)\n",
            "[python](../src/module.py) [rust](../src/lib.rs) [go](../src/main.go)\n",
            "[java](../src/Main.java) [typescript](../src/index.ts)\n",
            "[missing fragment](reference.md#absent)\n",
            "[ambiguous extension](dual#section)\n",
        ),
    )?;
    std::fs::write(
        &reference_path,
        "# Reference\n\n## Target section\n\nImportant behavior.\n",
    )?;
    std::fs::write(&index_path, "# Overview\n\nDirectory documentation.\n")?;
    std::fs::write(&ambiguous_markdown_path, "# Section\n")?;
    std::fs::write(&ambiguous_mdx_path, "# Section\n")?;
    std::fs::write(&skill_path, "# Agent\n\n## Rules\n\nUse exact evidence.\n")?;
    for (path, source) in code_paths.iter().zip([
        "def documented():\n    pass\n",
        "pub fn documented() {}\n",
        "package main\nfunc documented() {}\n",
        "class Main { void documented() {} }\n",
        "export function documented() {}\n",
    ]) {
        std::fs::write(&path.0, source)?;
    }

    let mut engine = Engine::default();
    let mut extractions = vec![
        engine.extract(&guide_path)?,
        engine.extract(&reference_path)?,
        engine.extract(&index_path)?,
        engine.extract(&ambiguous_markdown_path)?,
        engine.extract(&ambiguous_mdx_path)?,
        engine.extract(&skill_path)?,
    ];
    for (path, _) in &code_paths {
        extractions.push(engine.extract(path)?);
    }
    let merged = resolve_with_root(&extractions, &HashMap::new(), directory.path());
    let target_heading = merged
        .nodes
        .iter()
        .find(|node| {
            node.string("source_file").ends_with("docs/reference.md")
                && node.string("anchor_slug") == "target-section"
        })
        .ok_or("missing cross-document target heading")?;
    let overview_heading = merged
        .nodes
        .iter()
        .find(|node| {
            node.string("source_file").ends_with("docs/topic/README.md")
                && node.string("anchor_slug") == "overview"
        })
        .ok_or("missing directory index heading")?;
    let skill_heading = merged
        .nodes
        .iter()
        .find(|node| {
            node.string("source_file").ends_with("docs/agent.skill")
                && node.string("anchor_slug") == "rules"
        })
        .ok_or("missing skill heading")?;
    let resolved = merged
        .edges
        .iter()
        .filter(|edge| {
            edge.string("source_file").ends_with("docs/guide.md")
                && edge.string("relation") == "references"
                && edge.string("_document_target_resolution").is_empty()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        resolved.len(),
        7,
        "resolved={resolved:#?}; guide_edges={:#?}",
        merged
            .edges
            .iter()
            .filter(|edge| edge.string("source_file").ends_with("docs/guide.md"))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        resolved
            .iter()
            .filter(|edge| edge.target == target_heading.id)
            .count(),
        5
    );
    assert_eq!(
        resolved
            .iter()
            .filter(|edge| edge.target == overview_heading.id)
            .count(),
        1
    );
    assert_eq!(
        resolved
            .iter()
            .filter(|edge| edge.target == skill_heading.id)
            .count(),
        1
    );
    assert!(resolved.iter().all(|edge| {
        edge.string("rule") == "document-link-exact-target"
            && edge.string("resolution_rule") == "document-link-target-resolution"
            && !edge
                .attributes
                .contains_key(compass_model::provenance::ENDPOINT_REWRITE_RULES_ATTRIBUTE)
    }));
    let unresolved = merged
        .edges
        .iter()
        .filter(|edge| {
            edge.string("source_file").ends_with("docs/guide.md")
                && !edge.string("_document_target_resolution").is_empty()
        })
        .map(|edge| edge.string("_document_target_resolution"))
        .collect::<Vec<_>>();
    assert_eq!(unresolved, ["missing_fragment", "ambiguous_target"]);
    let code_file_ids = merged
        .nodes
        .iter()
        .filter(|node| {
            node.string("source_file").contains("src/") && node.string("symbol_kind") == "file"
        })
        .map(|node| node.id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        code_file_ids.len(),
        5,
        "missing language file inventory node"
    );
    assert_eq!(
        merged
            .edges
            .iter()
            .filter(|edge| {
                edge.string("source_file").ends_with("docs/guide.md")
                    && edge.string("relation") == "documents"
                    && code_file_ids.contains(edge.target.as_str())
            })
            .count(),
        5
    );
    let graph = compass_graph::build(&[merged], true, true, Some(directory.path()))?;
    let mut published = compass_graph::normalize_document_v1_with_inventory_best_effort(
        &graph,
        directory.path(),
        "test",
        None,
        {
            let mut inventory = [
                (&guide_path, "markdown"),
                (&reference_path, "markdown"),
                (&index_path, "markdown"),
                (&ambiguous_markdown_path, "markdown"),
                (&ambiguous_mdx_path, "markdown"),
                (&skill_path, "markdown"),
            ]
            .into_iter()
            .map(|(path, language)| compass_graph::InventoryEvidence {
                path: path.clone(),
                language: Some(language.to_owned()),
                producer: format!("compass.languages.{language}"),
                status: compass_model::code_graph::ExtractionStatus::Extracted,
                reason: None,
            })
            .collect::<Vec<_>>();
            inventory.extend(code_paths.iter().map(|(path, language)| {
                compass_graph::InventoryEvidence {
                    path: path.clone(),
                    language: Some((*language).to_owned()),
                    producer: format!("compass.languages.{language}"),
                    status: compass_model::code_graph::ExtractionStatus::Extracted,
                    reason: None,
                }
            }));
            inventory
        },
    )?;
    let published_links = published
        .document
        .links
        .iter()
        .filter(|edge| {
            edge.kind.as_str() == "references"
                && edge
                    .relationship_site
                    .as_ref()
                    .is_some_and(|site| site.file == "docs/guide.md")
        })
        .count();
    assert_eq!(published_links, 7, "published={:#?}", published.document);
    assert_eq!(
        published
            .document
            .links
            .iter()
            .filter(|edge| {
                edge.kind.as_str() == "documents"
                    && edge
                        .relationship_site
                        .as_ref()
                        .is_some_and(|site| site.file == "docs/guide.md")
            })
            .count(),
        5
    );
    apply_inference_level(&mut published.document, InferenceLevel::Low);
    assert_eq!(
        published
            .document
            .links
            .iter()
            .filter(|edge| {
                matches!(edge.kind.as_str(), "references" | "documents")
                    && edge
                        .relationship_site
                        .as_ref()
                        .is_some_and(|site| site.file == "docs/guide.md")
            })
            .count(),
        12,
        "exact document relationships must survive the default low inference level"
    );
    Ok(())
}

#[test]
fn collection_resolution_consumes_each_rust_evidence_batch_once() -> Result<(), Box<dyn Error>> {
    let mut engine = Engine::default();
    let left_source =
        b"struct Left {} impl Left { fn new() -> Self { Self {} } } fn left() { Left::new(); }";
    let right_source =
        b"struct Right {} impl Right { fn new() -> Self { Self {} } } fn right() { Right::new(); }";
    let left = engine.extract_source(Path::new("src/left.rs"), left_source)?;
    let right = engine.extract_source(Path::new("src/right.rs"), right_source)?;
    let sources = HashMap::from([
        (
            "src/left.rs".to_owned(),
            String::from_utf8(left_source.to_vec())?,
        ),
        (
            "src/right.rs".to_owned(),
            String::from_utf8(right_source.to_vec())?,
        ),
    ]);

    let merged = resolve(&[left, right], &sources);
    assert!(merged.semantic_evidence.is_none());
    assert!(merged.raw_calls.as_ref().is_some_and(Vec::is_empty));
    for (caller_name, target_name) in [
        ("crate::left::left", "crate::left::Left::new"),
        ("crate::right::right", "crate::right::Right::new"),
    ] {
        let caller = merged
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == caller_name)
            .ok_or_else(|| format!("missing {caller_name}"))?;
        let target = merged
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == target_name)
            .ok_or_else(|| format!("missing {target_name}"))?;
        assert!(merged.edges.iter().any(|edge| {
            edge.source == caller.id
                && edge.target == target.id
                && edge.string("relation") == "calls"
                && edge.string("extractor") == "compass.resolve.rust.universal"
        }));
    }
    Ok(())
}

#[test]
fn rust_call_resolution_distinguishes_same_named_fields_and_methods() -> Result<(), Box<dyn Error>>
{
    let source = br#"
struct Entry { path: String }
impl Entry { fn path(&self) -> &str { &self.path } }
fn read(entry: &Entry) { entry.path(); }
"#;
    let source_file = "src/lib.rs";
    let extracted = Engine::default().extract_source(Path::new(source_file), source)?;
    let call = extracted
        .semantic_evidence
        .as_ref()
        .and_then(|evidence| {
            evidence.candidates.iter().find(|candidate| {
                candidate.relation == CandidateRelation::Calls
                    && candidate.target_spelling == "path"
            })
        })
        .ok_or("missing path call evidence")?;
    assert_eq!(
        call.constraints.allowed_target_kinds,
        ["enum_member", "function", "method", "struct"]
    );
    let merged = resolve(
        &[extracted],
        &HashMap::from([(source_file.to_owned(), String::from_utf8(source.to_vec())?)]),
    );
    let read = merged
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::read")
        .ok_or("missing read")?;
    let named = merged
        .nodes
        .iter()
        .filter(|node| node.string("qualified_name") == "crate::Entry::path")
        .collect::<Vec<_>>();
    assert_eq!(named.len(), 2, "nodes={named:#?}");
    assert_ne!(named[0].id, named[1].id, "nodes={named:#?}");
    let call_edges = merged
        .edges
        .iter()
        .filter(|edge| edge.source == read.id && edge.string("relation") == "calls")
        .collect::<Vec<_>>();
    let targets = call_edges
        .iter()
        .filter_map(|edge| merged.nodes.iter().find(|node| node.id == edge.target))
        .collect::<Vec<_>>();
    assert_eq!(targets.len(), 1, "edges={:#?}", merged.edges);
    assert_eq!(
        targets[0].string("symbol_kind"),
        "method",
        "target={:#?} edges={call_edges:#?}",
        targets[0],
    );
    assert_eq!(targets[0].string("qualified_name"), "crate::Entry::path");
    Ok(())
}

#[test]
fn rust_generic_impl_bound_calls_resolve_to_the_trait_method() -> Result<(), Box<dyn Error>> {
    let source = br#"
trait Render { fn render(&self); }
struct Wrapper<T> { value: T }
impl<T> Wrapper<T>
where
    T: Render,
{
    fn invoke(&self, value: T) { value.render(); self.value.render(); }
}
"#;
    let source_file = "src/lib.rs";
    let extracted = Engine::default().extract_source(Path::new(source_file), source)?;
    let merged = resolve(
        &[extracted],
        &HashMap::from([(source_file.to_owned(), String::from_utf8(source.to_vec())?)]),
    );
    let invoke = merged
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::Wrapper::invoke")
        .ok_or("missing generic impl method")?;
    let render = merged
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::Render::render")
        .ok_or("missing trait method")?;
    let calls = merged
        .edges
        .iter()
        .filter(|edge| edge.source == invoke.id && edge.string("relation") == "calls")
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 2, "calls={calls:#?}");
    assert!(
        calls.iter().all(|edge| edge.target == render.id),
        "calls={calls:#?}"
    );
    Ok(())
}

#[test]
fn rust_import_aliases_resolve_cross_file_types_functions_and_methods() -> Result<(), Box<dyn Error>>
{
    let mut engine = Engine::default();
    let api_source = br#"
pub trait Render { fn render(&self); }
pub struct Widget {}
impl Widget { pub fn new() -> Self { Self {} } }
impl Render for Widget { fn render(&self) {} }
pub fn make() -> Widget { Widget::new() }
"#;
    let caller_source = br#"
mod api;
use crate::api::{make as build_widget, Widget};
fn run() { build_widget(); Widget::new(); }
"#;
    let api = engine.extract_source(Path::new("src/api.rs"), api_source)?;
    let caller = engine.extract_source(Path::new("src/lib.rs"), caller_source)?;
    let sources = HashMap::from([
        (
            "src/api.rs".to_owned(),
            String::from_utf8(api_source.to_vec())?,
        ),
        (
            "src/lib.rs".to_owned(),
            String::from_utf8(caller_source.to_vec())?,
        ),
    ]);
    let merged = resolve(&[api, caller], &sources);
    let run = merged
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::run")
        .ok_or("missing run")?;
    for (target_name, resolution_rule) in [
        ("crate::api::make", "explicit-binding"),
        ("crate::api::Widget::new", "member-binding"),
    ] {
        let target = merged
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == target_name)
            .ok_or_else(|| format!("missing {target_name}: {:#?}", merged.nodes))?;
        assert!(merged.edges.iter().any(|edge| {
            edge.source == run.id
                && edge.target == target.id
                && edge.string("relation") == "calls"
                && edge.string("resolution_rule") == resolution_rule
        }));
    }
    let widget = merged
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::api::Widget")
        .ok_or("missing Widget")?;
    let render = merged
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::api::Render")
        .ok_or("missing Render")?;
    assert!(merged.edges.iter().any(|edge| {
        edge.source == widget.id
            && edge.target == render.id
            && edge.string("relation") == "implements"
    }));
    Ok(())
}

#[test]
fn java_imports_receivers_and_overload_arity_resolve_on_the_universal_path()
-> Result<(), Box<dyn Error>> {
    let mut engine = Engine::default();
    let repository_source = br#"package org.example.data;
public class Repository {
    public Result load(String key) { return new Result(); }
    public Result load(String key, int limit) { return new Result(); }
}
class Result {}
"#;
    let service_source = br#"package org.example.app;
import org.example.data.Repository;
public class Service {
    private final Repository repository;
    public Service(Repository repository) { this.repository = repository; }
    public void run() { repository.load("one"); }
}
"#;
    let repository = engine.extract_source(
        Path::new("src/main/java/org/example/data/Repository.java"),
        repository_source,
    )?;
    let service = engine.extract_source(
        Path::new("src/main/java/org/example/app/Service.java"),
        service_source,
    )?;
    assert!(repository.nodes.is_empty() && repository.edges.is_empty());
    assert!(service.nodes.is_empty() && service.edges.is_empty());
    let sources = HashMap::from([
        (
            "src/main/java/org/example/data/Repository.java".to_owned(),
            String::from_utf8(repository_source.to_vec())?,
        ),
        (
            "src/main/java/org/example/app/Service.java".to_owned(),
            String::from_utf8(service_source.to_vec())?,
        ),
    ]);
    let merged = resolve(&[repository, service], &sources);
    assert!(merged.error.is_none(), "{:#?}", merged.error);
    let run = merged
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "org.example.app.Service::run")
        .ok_or("missing Service::run")?;
    let loads = merged
        .nodes
        .iter()
        .filter(|node| node.string("qualified_name") == "org.example.data.Repository::load")
        .collect::<Vec<_>>();
    assert_eq!(loads.len(), 2);
    let one_argument_load = loads
        .iter()
        .find(|node| node.string("signature") == "load(String)")
        .ok_or("missing one-argument overload")?;
    assert!(
        merged.edges.iter().any(|edge| {
            edge.source == run.id
                && edge.target == one_argument_load.id
                && edge.string("relation") == "calls"
                && edge.string("resolution_rule") == "explicit-binding"
        }),
        "one-argument overload was not selected"
    );
    assert!(!merged.edges.iter().any(|edge| {
        edge.source == run.id
            && loads
                .iter()
                .any(|load| load.id == edge.target && load.id != one_argument_load.id)
            && edge.string("relation") == "calls"
    }));
    Ok(())
}

#[test]
fn java_contains_references_and_declarations_survive_universal_publication()
-> Result<(), Box<dyn Error>> {
    let source = br#"package demo.catalog;

interface Repository {
    void enqueue(Catalog item);
}

class Catalog {}

class Worker extends Catalog implements Repository {
    public void enqueue(Catalog item) {
        this.track(item);
    }

    void track(Catalog item) {}
}

class Service {
    private final Catalog catalog;

    public Service(Catalog catalog) {
        this.catalog = catalog;
    }

    public Catalog run(Catalog item) {
        return new Catalog();
    }
}
"#;
    let source_file = "src/main/java/demo/catalog/Service.java";
    let extracted = Engine::default()
        .extract_source_combined(Path::new(source_file), source_file, source)?
        .graph;

    assert!(extracted.nodes.is_empty());
    assert!(extracted.edges.is_empty());
    assert!(extracted.raw_calls.is_none());
    let evidence = extracted
        .semantic_evidence
        .as_ref()
        .ok_or("missing Java universal evidence")?;
    validate_evidence(evidence, EvidenceLimits::default())?;
    assert_eq!(evidence.pipeline.id, "compass.java");
    assert_eq!(evidence.pipeline.version, 3);

    let declaration_id = |qualified_name: &str| {
        evidence
            .declarations
            .iter()
            .find(|declaration| declaration.qualified_name == qualified_name)
            .map(|declaration| declaration.id.as_str())
    };
    let service_id = declaration_id("demo.catalog.Service").ok_or("missing Service evidence")?;
    let worker_id = declaration_id("demo.catalog.Worker").ok_or("missing Worker evidence")?;
    let run_id = declaration_id("demo.catalog.Service::run").ok_or("missing run evidence")?;
    declaration_id("demo.catalog.Service::<init>").ok_or("missing Service constructor evidence")?;
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Contains
            && candidate.source_declaration_id == service_id
            && candidate.target_spelling == "run"
    }));
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Contains
            && candidate.source_declaration_id == service_id
            && candidate.target_spelling == "<init>"
            && candidate.constraints.exact_target_declaration_id.is_some()
            && candidate.constraints.argument_count == Some(1)
    }));
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Extends
            && candidate.source_declaration_id == worker_id
            && candidate.target_spelling == "Catalog"
    }));
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Implements
            && candidate.source_declaration_id == worker_id
            && candidate.target_spelling == "Repository"
    }));
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::References
            && candidate.source_declaration_id == run_id
            && candidate.target_spelling == "Catalog"
    }));

    let resolved = resolve(
        &[extracted],
        &HashMap::from([(source_file.to_owned(), String::from_utf8(source.to_vec())?)]),
    );
    assert!(resolved.error.is_none(), "{:#?}", resolved.error);
    let node = |qualified_name: &str| {
        resolved
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == qualified_name)
    };
    let service = node("demo.catalog.Service").ok_or("missing published Service")?;
    let worker = node("demo.catalog.Worker").ok_or("missing published Worker")?;
    let catalog = node("demo.catalog.Catalog").ok_or("missing published Catalog")?;
    let repository = node("demo.catalog.Repository").ok_or("missing published Repository")?;
    let run = node("demo.catalog.Service::run").ok_or("missing published run")?;
    let constructor =
        node("demo.catalog.Service::<init>").ok_or("missing published Service constructor")?;
    for published in [service, worker, catalog, repository, run, constructor] {
        assert_eq!(published.string("language"), "java");
        assert_eq!(
            published.string("extractor"),
            "compass.languages.java.universal"
        );
        assert_eq!(published.string("source_file"), source_file);
        assert!(!published.string("evidence_declaration_id").is_empty());
    }
    for (source_id, target_id, relation) in [
        (service.id.as_str(), run.id.as_str(), "contains"),
        (service.id.as_str(), constructor.id.as_str(), "contains"),
        (worker.id.as_str(), catalog.id.as_str(), "inherits"),
        (worker.id.as_str(), repository.id.as_str(), "implements"),
        (run.id.as_str(), catalog.id.as_str(), "references"),
    ] {
        assert!(
            resolved.edges.iter().any(|edge| {
                edge.source == source_id
                    && edge.target == target_id
                    && edge.string("relation") == relation
                    && edge.string("extractor") == "compass.resolve.java.universal"
            }),
            "missing {relation} edge {source_id} -> {target_id}"
        );
    }
    Ok(())
}

#[test]
fn java_same_arity_overloads_keep_exact_ownership() -> Result<(), Box<dyn Error>> {
    let source = br#"package demo;
class Parser {
    String parse(String value) { return value; }
    String parse(Object value) { return value.toString(); }
}
"#;
    let source_file = "src/main/java/demo/Parser.java";
    let extracted = Engine::default()
        .extract_source_combined(Path::new(source_file), source_file, source)?
        .graph;
    let evidence = extracted
        .semantic_evidence
        .as_ref()
        .ok_or("missing Java universal evidence")?;
    let overloads = evidence
        .declarations
        .iter()
        .filter(|declaration| declaration.qualified_name == "demo.Parser::parse")
        .collect::<Vec<_>>();
    assert_eq!(overloads.len(), 2);
    assert!(overloads.iter().all(|overload| {
        evidence.candidates.iter().any(|candidate| {
            candidate.relation == CandidateRelation::Contains
                && candidate.constraints.exact_target_declaration_id.as_deref()
                    == Some(overload.id.as_str())
        })
    }));

    let resolved = resolve(
        &[extracted],
        &HashMap::from([(source_file.to_owned(), String::from_utf8(source.to_vec())?)]),
    );
    assert!(resolved.error.is_none(), "{:#?}", resolved.error);
    let parser = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "demo.Parser")
        .ok_or("missing Parser declaration")?;
    let published_overloads = resolved
        .nodes
        .iter()
        .filter(|node| node.string("qualified_name") == "demo.Parser::parse")
        .collect::<Vec<_>>();
    assert_eq!(published_overloads.len(), 2);
    assert!(published_overloads.iter().all(|overload| {
        resolved.edges.iter().any(|edge| {
            edge.source == parser.id
                && edge.target == overload.id
                && edge.string("relation") == "contains"
        })
    }));
    Ok(())
}

#[test]
fn java_exact_argument_types_select_one_overload_and_unknown_arguments_fail_closed()
-> Result<(), Box<dyn Error>> {
    let source = br#"package demo;
class Selector {
    String choose(String value) { return value; }
    String choose(Object value) { return value.toString(); }
    Object factory() { return new Object(); }
    void run() {
        choose("exact");
        choose(factory());
    }
}
"#;
    let source_file = "src/main/java/demo/Selector.java";
    let extracted = Engine::default()
        .extract_source_combined(Path::new(source_file), source_file, source)?
        .graph;
    let evidence = extracted
        .semantic_evidence
        .as_ref()
        .ok_or("missing Java universal evidence")?;
    let choose_candidates = evidence
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.relation == CandidateRelation::Calls && candidate.target_spelling == "choose"
        })
        .collect::<Vec<_>>();
    assert_eq!(choose_candidates.len(), 2);
    assert!(choose_candidates.iter().any(|candidate| {
        candidate.constraints.argument_types == [Some("java.lang.String".to_owned())]
    }));
    assert!(
        choose_candidates
            .iter()
            .any(|candidate| candidate.constraints.argument_types == [None])
    );

    let resolved = resolve(
        &[extracted],
        &HashMap::from([(source_file.to_owned(), String::from_utf8(source.to_vec())?)]),
    );
    assert!(resolved.error.is_none(), "{:#?}", resolved.error);
    let run = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "demo.Selector::run")
        .ok_or("missing run declaration")?;
    let selected = resolved
        .edges
        .iter()
        .filter(|edge| edge.source == run.id && edge.string("relation") == "calls")
        .filter_map(|edge| resolved.nodes.iter().find(|node| node.id == edge.target))
        .filter(|target| target.string("qualified_name") == "demo.Selector::choose")
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), 1, "selected={selected:#?}");
    assert_eq!(selected[0].string("signature"), "choose(String)");
    Ok(())
}

#[test]
fn java_proven_conversions_select_the_unique_most_specific_overload() -> Result<(), Box<dyn Error>>
{
    let source = br#"package demo;
import vendor.External;
class Base {}
class Child extends Base {}
class Other {}
class Selector {
    String select(Object value) { return "object"; }
    String select(Base value) { return "base"; }
    Object factory() { return new Object(); }
    void run(Other other, Child child, External external) {
        select(other);
        select(child);
        select(1);
        select(new int[1]);
        select(factory());
        select(external);
    }
}
"#;
    let source_file = "src/main/java/demo/Selector.java";
    let extracted = Engine::default()
        .extract_source_combined(Path::new(source_file), source_file, source)?
        .graph;
    let resolved = resolve(
        &[extracted],
        &HashMap::from([(source_file.to_owned(), String::from_utf8(source.to_vec())?)]),
    );
    assert!(resolved.error.is_none(), "{:#?}", resolved.error);
    let run = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "demo.Selector::run")
        .ok_or("missing run declaration")?;
    let selected = resolved
        .edges
        .iter()
        .filter(|edge| edge.source == run.id && edge.string("relation") == "calls")
        .filter_map(|edge| resolved.nodes.iter().find(|node| node.id == edge.target))
        .filter(|target| target.string("qualified_name") == "demo.Selector::select")
        .map(|target| target.string("signature"))
        .collect::<Vec<_>>();
    assert_eq!(
        selected,
        [
            "select(Object)",
            "select(Base)",
            "select(Object)",
            "select(Object)"
        ],
        "selected={selected:#?}"
    );
    Ok(())
}

#[test]
fn java_collection_source_paths_keep_duplicate_class_copies_distinct() -> Result<(), Box<dyn Error>>
{
    let source = br#"package org.example.shared;
public class Duplicate { public void run() {} }
"#;
    let first_source = "module-a/src/main/java/org/example/shared/Duplicate.java";
    let second_source = "module-b/src/test/java/org/example/shared/Duplicate.java";
    let mut engine = Engine::default();
    let first = engine
        .extract_source_combined(
            Path::new("/repo/module-a/src/main/java/org/example/shared/Duplicate.java"),
            first_source,
            source,
        )?
        .graph;
    let second = engine
        .extract_source_combined(
            Path::new("/repo/module-b/src/test/java/org/example/shared/Duplicate.java"),
            second_source,
            source,
        )?
        .graph;
    let sources = HashMap::from([
        (first_source.to_owned(), String::from_utf8(source.to_vec())?),
        (
            second_source.to_owned(),
            String::from_utf8(source.to_vec())?,
        ),
    ]);

    let merged = resolve(&[first, second], &sources);
    assert!(merged.error.is_none(), "{:#?}", merged.error);
    let duplicates = merged
        .nodes
        .iter()
        .filter(|node| node.string("qualified_name") == "org.example.shared.Duplicate")
        .collect::<Vec<_>>();
    assert_eq!(duplicates.len(), 2, "{:#?}", merged.nodes);
    assert_ne!(duplicates[0].id, duplicates[1].id);
    assert_eq!(
        duplicates
            .iter()
            .map(|node| node.string("source_file"))
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from([first_source.to_owned(), second_source.to_owned()])
    );
    Ok(())
}
