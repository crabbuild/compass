use compass_core::{BuildOptions, GraphStorage, build_local_graph};
use compass_model::code_graph::GraphDocument;
use compass_model::query_contract::{
    DiscoveryDirection, DiscoveryLimits, DiscoveryQueryRequest, DiscoveryTraversal,
};
use compass_model::source_query::{SOURCE_QUERY_FILE, SourceQueryArtifact};
use compass_query::{EngineSelection, open_with_engine};
use std::error::Error;
use std::fs;
use std::path::Path;
fn request() -> DiscoveryQueryRequest {
    DiscoveryQueryRequest {
        question: "Where is payload handled?".to_owned(),
        direction: DiscoveryDirection::Auto,
        relation_contexts: Vec::new(),
        scope: Vec::new(),
        traversal: DiscoveryTraversal::Bfs,
        include_heuristic: false,
        limits: DiscoveryLimits::default(),
    }
}
fn options(root: &Path, output: &Path) -> BuildOptions {
    let mut o = BuildOptions::new(root);
    o.output_root = Some(output.to_path_buf());
    o.graph_storage = GraphStorage::Json;
    o.no_cluster = true;
    o.no_viz = true;
    o.max_workers = Some(1);
    o
}

#[test]
fn role_recall_keeps_parameter_noun_topics() -> Result<(), Box<dyn Error>> {
    let d = tempfile::tempdir()?;
    let root = d.path().join("repo");
    fs::create_dir(&root)?;
    fs::write(
        root.join("sample.py"),
        concat!(
            "def decode(block):\n    return block\n\n",
            "def check(permits):\n    return permits\n\n",
            "def decode_lure(value=\"block\"):\n    return value\n\n",
            "def enclosing(value):\n    def hidden(block):\n        return block\n    return hidden(value)\n\n",
            "def guard(value):\n    if value:\n        raise ValueError('retired account access blocked and permitted')\n    return value\n",
        ),
    )?;
    let mut o = options(&root, &d.path().join("out"));
    o.graph_storage = GraphStorage::Sqlite;
    let built = build_local_graph(&o)?;
    let graph = built.output_dir.join("graph.json");
    for mode in [EngineSelection::Json, EngineSelection::Store] {
        let engine = open_with_engine(
            &graph,
            None,
            &d.path().join(format!("query-{mode:?}")),
            mode,
        )?;
        for (question, name, missing) in [
            ("Where are memory blocks decoded?", "decode", "memory"),
            ("Where are temporary permits checked?", "check", "temporary"),
        ] {
            let mut query = request();
            query.question = question.to_owned();
            let answer = engine.discover_ranked(query.clone())?;
            let target = answer
                .candidates
                .iter()
                .find(|c| c.node.name.trim_end_matches("()") == name)
                .ok_or_else(|| format!("noun topic lost: {question}"))?;
            assert_eq!(target.relevance, compass_query::RelevanceBand::Low);
            assert!(
                target
                    .reasons
                    .iter()
                    .any(|r| r.contains("typed AST-owned parameter"))
            );
            assert!(
                !target
                    .reasons
                    .iter()
                    .any(|r| r.contains("inspection preference"))
            );
            assert!(!answer.candidates.iter().any(|c| matches!(
                c.node.name.trim_end_matches("()"),
                "decode_lure" | "enclosing" | "guard"
            )));
            assert!(
                target
                    .reasons
                    .iter()
                    .any(|r| r.contains("missing owner-specific context") && r.contains(missing))
            );
        }
        let mut collision = request();
        collision.question = "Where are blocks blocked?".to_owned();
        let answer = engine.discover_ranked(collision)?;
        let target = answer
            .candidates
            .iter()
            .find(|c| c.node.name.trim_end_matches("()") == "decode")
            .ok_or("raw noun/predicate collision lost noun witness")?;
        assert_eq!(target.relevance, compass_query::RelevanceBand::Low);
        assert!(target.reasons.iter().any(|r| r.contains("blocks, blocked")
            && r.contains("2 raw query occurrences")
            && r.contains("does not verify")));
        assert!(
            !answer
                .candidates
                .iter()
                .any(|c| c.node.name.trim_end_matches("()") == "guard")
        );
        let mut repeated = request();
        repeated.question = "Where are blocks blocks blocked?".to_owned();
        let repeated = engine.discover_ranked(repeated)?;
        let target = repeated
            .candidates
            .iter()
            .find(|c| c.node.name.trim_end_matches("()") == "decode")
            .ok_or("repeated raw noun witness lost")?;
        assert!(target.reasons.iter().any(
            |r| r.contains("blocks, blocks, blocked") && r.contains("3 raw query occurrences")
        ));
        for question in [
            "Where are orbital plasmid launches blocked?",
            "Where are orbital plasmid launches permitted?",
        ] {
            let mut query = request();
            query.question = question.to_owned();
            assert!(
                engine.discover_ranked(query)?.candidates.is_empty(),
                "unrelated verb hit: {question}"
            );
        }
    }
    Ok(())
}

#[test]
fn role_recall_keeps_named_qualifier_owners() -> Result<(), Box<dyn Error>> {
    let d = tempfile::tempdir()?;
    let root = d.path().join("repo");
    fs::create_dir(&root)?;
    let mut source = String::new();
    for name in ["New", "All", "Only", "Made"] {
        source.push_str(&format!(
            "class {name}:\n    def encode(self, value):\n        return value\n\n"
        ));
    }
    source.push_str("def guard(value):\n    if value:\n        raise ValueError('New encode retired account access rejected')\n    return value\n");
    fs::write(root.join("sample.py"), source)?;
    let mut o = options(&root, &d.path().join("out"));
    o.graph_storage = GraphStorage::Sqlite;
    let built = build_local_graph(&o)?;
    let graph = built.output_dir.join("graph.json");
    for mode in [EngineSelection::Json, EngineSelection::Store] {
        let engine = open_with_engine(
            &graph,
            None,
            &d.path().join(format!("query-{mode:?}")),
            mode,
        )?;
        for name in ["New", "All", "Only", "Made"] {
            let mut query = request();
            query.question = format!("Where does {name} encode orbital plasmids?");
            let answer = engine.discover_ranked(query.clone())?;
            let document = GraphDocument::load(&graph)?;
            let expected = document
                .nodes
                .iter()
                .find(|node| {
                    node.kind.is_callable()
                        && node.name == ".encode()"
                        && node.qualified_name.ends_with(&format!("{name}::encode"))
                })
                .ok_or("missing actual named native method")?;
            let target = answer
                .candidates
                .iter()
                .find(|c| c.node.id == expected.id)
                .ok_or_else(|| format!("real named owner lost: {name}"))?;
            assert_eq!(target.relevance, compass_query::RelevanceBand::Low);
            assert!(
                !target
                    .reasons
                    .iter()
                    .any(|r| r.contains("inspection preference"))
            );
            assert!(
                !answer
                    .candidates
                    .iter()
                    .any(|c| c.node.name.trim_end_matches("()") == "guard")
            );
        }
        let mut query = request();
        query.question = "Where are new orbital plasmid launches rejected?".to_owned();
        assert!(engine.discover_ranked(query)?.candidates.is_empty());
    }
    Ok(())
}

#[test]
fn decision_verbs_require_independent_owner_topics() -> Result<(), Box<dyn Error>> {
    let d = tempfile::tempdir()?;
    let root = d.path().join("repo");
    fs::create_dir(&root)?;
    let spellings = [
        "reject",
        "rejects",
        "rejected",
        "rejecting",
        "accept",
        "accepts",
        "accepted",
        "accepting",
        "deny",
        "denies",
        "denied",
        "denying",
        "allow",
        "allows",
        "allowed",
        "allowing",
        "block",
        "blocks",
        "blocked",
        "blocking",
        "permit",
        "permits",
        "permitted",
        "permitting",
        "refuse",
        "refuses",
        "refused",
        "refusing",
    ];
    for (index, spelling) in spellings.iter().enumerate() {
        fs::write(
            root.join(format!("sample_{index}.py")),
            format!(
                "def opaque_{index}(value):\n    if value:\n        raise ValueError('retired account access {spelling}')\n    return value\n"
            ),
        )?;
    }
    let mut o = options(&root, &d.path().join("out"));
    o.graph_storage = GraphStorage::Sqlite;
    let built = build_local_graph(&o)?;
    let graph = built.output_dir.join("graph.json");
    let mut expected = Vec::new();
    for mode in [EngineSelection::Json, EngineSelection::Store] {
        let engine = open_with_engine(
            &graph,
            None,
            &d.path().join(format!("query-{mode:?}")),
            mode,
        )?;
        for (index, spelling) in spellings.iter().enumerate() {
            let mut query = request();
            query.question = format!("Where are orbital plasmid launches {spelling}?");
            let answer = engine.discover_ranked(query.clone())?;
            assert!(
                answer.candidates.is_empty(),
                "action-only hit for {spelling}: {:?}",
                answer.candidates
            );
            assert!(
                engine.discover(query.clone())?.seeds.is_empty(),
                "default action-only hit: {spelling}"
            );
            let search = engine.search(compass_model::query_contract::SearchRequest {
                query: query.question.clone(),
                limits: compass_model::query_contract::CodeQueryLimits::default(),
            })?;
            assert!(
                search.results.is_empty(),
                "search action-only hit: {spelling}"
            );
            query.question = format!("Where are retired account access attempts {spelling}?");
            let positive = engine.discover_ranked(query)?;
            let target = positive
                .candidates
                .iter()
                .find(|c| c.node.name.trim_end_matches("()") == format!("opaque_{index}"))
                .ok_or_else(|| format!("literal topic owner missing for {spelling}"))?;
            assert_eq!(target.relevance, compass_query::RelevanceBand::Low);
            assert!(target.matched_fields.iter().any(|f| f == "body_code"));
            assert!(!target.supporting_sources.is_empty());
            assert!(
                !target
                    .reasons
                    .iter()
                    .any(|r| r.contains("inspection preference"))
            );
            if mode == EngineSelection::Json {
                expected.push(positive.candidates);
            } else {
                assert_eq!(positive.candidates, expected[index]);
            }
        }
    }
    Ok(())
}

#[test]
fn independent_topic_admission_preserves_partial_and_exact_inspection() -> Result<(), Box<dyn Error>>
{
    let d = tempfile::tempdir()?;
    let root = d.path().join("repo");
    fs::create_dir(&root)?;
    fs::write(
        root.join("sample.py"),
        concat!(
            "def guard(value):\n    if value:\n        raise ValueError('retired account access rejected')\n    return value\n\n",
            "def counterpart(value):\n    if value:\n        raise ValueError('retired account access accepted')\n    return value\n\n",
            "def get(value):\n    if value:\n        raise ValueError('retired account access rejected')\n    return value\n\n",
            "def enclosing(value):\n    def hidden():\n        raise ValueError('orphan administrator tokens rejected')\n    return hidden()\n\n",
            "def reject(value):\n    return value\n\n",
            "def encode(value):\n    return rejected(value)\n\n",
            "def load_only(rejected):\n    return rejected\n\n",
            "def profile_state(rejected):\n    return rejected\n",
        ),
    )?;
    let mut o = options(&root, &d.path().join("out"));
    o.graph_storage = GraphStorage::Sqlite;
    let built = build_local_graph(&o)?;
    let graph = built.output_dir.join("graph.json");
    let questions = [
        "Where are orbital plasmid launches rejected?",
        "Where does lookup reject orbital plasmid launches?",
        "Where are retired account access attempts accepted?",
        "Where are retired account access attempts elegantly dismissed?",
        "Where are OrbitalPlasmid retired account access attempts rejected?",
        "Where are `orbital` retired account access attempts rejected?",
        "Where are orphan administrator tokens rejected?",
        "Where is reject?",
        "Which Python functions are defined?",
        "Find `guard`.",
        "Find `reject`.",
        "Where is reject encoded?",
        "Where are load-only rejected?",
        "Where is accepted?",
        "Where are only orbital plasmid launches rejected?",
        "Where are profile-state rejected?",
    ];
    let mut expected = Vec::new();
    for mode in [EngineSelection::Json, EngineSelection::Store] {
        let engine = open_with_engine(
            &graph,
            None,
            &d.path().join(format!("query-{mode:?}")),
            mode,
        )?;
        for (index, question) in questions.iter().enumerate() {
            let mut query = request();
            query.question = (*question).to_owned();
            let answer = engine.discover_ranked(query)?;
            let named = |name: &str| {
                answer
                    .candidates
                    .iter()
                    .find(|c| c.node.name.trim_end_matches("()") == name)
            };
            match index {
                0 | 1 | 4 | 5 | 14 => assert!(
                    answer.candidates.is_empty(),
                    "{question}: {:?}",
                    answer.candidates
                ),
                2 => {
                    let actual = named("counterpart").ok_or("accepted-word owner missing")?;
                    let opposite = named("guard").ok_or("partial topic pointer missing")?;
                    assert!(actual.matched_terms.iter().any(|t| t == "accept"));
                    assert!(!opposite.matched_terms.iter().any(|t| t == "accept"));
                    assert_eq!(opposite.relevance, compass_query::RelevanceBand::Low);
                    assert!(
                        opposite
                            .reasons
                            .iter()
                            .any(|r| r.contains("missing owner-specific context: accept"))
                    );
                    assert!(
                        !opposite
                            .reasons
                            .iter()
                            .any(|r| r.contains("Vocabulary fallback")
                                || r.contains("inspection preference"))
                    );
                }
                3 => {
                    let target = named("guard").ok_or("ordinary paraphrase lost its real topic")?;
                    assert_eq!(target.relevance, compass_query::RelevanceBand::Low);
                    assert!(
                        target
                            .reasons
                            .iter()
                            .any(|r| r.contains("missing owner-specific context")
                                && r.contains("elegantly"))
                    );
                }
                6 => {
                    assert!(named("hidden").is_some());
                    assert!(named("enclosing").is_none());
                    assert!(named("guard").is_none());
                }
                7 => assert!(
                    named("reject").is_some(),
                    "exact predicate-name inspection lost"
                ),
                8 => assert!(!answer.candidates.is_empty(), "pure facet listing lost"),
                9 | 10 => {
                    let name = if index == 9 { "guard" } else { "reject" };
                    let target = named(name).ok_or("named identity missing")?;
                    // Natural-language Find uses the existing structural seed
                    // contract; it need not report a broad exact_name field.
                    let mut exact_query = request();
                    exact_query.question = target.node.id.clone();
                    let exact = engine.discover_ranked(exact_query)?;
                    assert_eq!(exact.candidates.len(), 1);
                    assert_eq!(exact.candidates[0].node.id, target.node.id);
                    assert!(
                        exact
                            .discovery
                            .seeds
                            .iter()
                            .any(|seed| seed.node_id == target.node.id
                                && seed.score_tier
                                    == compass_model::query_contract::DiscoveryScoreTier::ExactId)
                    );
                }
                11 | 12 | 15 => {
                    let name = match index {
                        11 => "encode",
                        12 => "load_only",
                        _ => "profile_state",
                    };
                    let target = named(name).ok_or("predicate-only inspection lost")?;
                    assert!(
                        !target
                            .reasons
                            .iter()
                            .any(|r| r.contains("inspection preference")),
                        "predicate supplied independent preference: {target:?}"
                    );
                }
                13 => assert!(
                    named("counterpart").is_some(),
                    "non-exact predicate-only inspection lost"
                ),
                _ => return Err("unhandled control".into()),
            }
            if mode == EngineSelection::Json {
                expected.push(answer.candidates);
            } else {
                assert_eq!(
                    answer.candidates, expected[index],
                    "backend mismatch: {question}"
                );
            }
        }
    }
    Ok(())
}

#[test]
fn distinct_predicates_keep_real_nouns_without_borrowing_defaults_types_or_nested_sites()
-> Result<(), Box<dyn Error>> {
    let d = tempfile::tempdir()?;
    let root = d.path().join("repo");
    fs::create_dir(&root)?;
    let source = concat!(
        "def decode(block):\n    return block\n\n",
        "def decode_after(x=use(block), block=0):\n    return block\n\n",
        "def decode_default(value=\"block\"):\n    return value\n\n",
        "def decode_type(value: Block):\n    return value\n\n",
        "def enclosing(value):\n    def hidden(block):\n        return block\n    return hidden(value)\n\n",
        "def check(permits):\n    return permits\n\n",
        "def check_default(value=\"permits\"):\n    return value\n\n",
        "def guard(value):\n    if value:\n        raise ValueError('blocks rejected, permits allowed, blocks blocked')\n    return value\n",
    );
    fs::write(root.join("sample.py"), source)?;
    let mut o = options(&root, &d.path().join("out"));
    o.graph_storage = GraphStorage::Sqlite;
    let built = build_local_graph(&o)?;
    let graph = built.output_dir.join("graph.json");
    let document = GraphDocument::load(&graph)?;
    let body: SourceQueryArtifact =
        serde_json::from_slice(&fs::read(built.output_dir.join(SOURCE_QUERY_FILE))?)?;
    let expected = document
        .nodes
        .iter()
        .find(|n| n.name == "decode_after()")
        .ok_or("missing actual after owner")?;
    let binding = body
        .owners
        .iter()
        .find(|o| o.node_id == expected.id)
        .and_then(|o| o.parameter_bindings.as_ref())
        .and_then(|b| b.sites.iter().find(|s| s.text == "block"))
        .ok_or("missing true later binding")?;
    assert_eq!(
        binding.source.start_byte as usize,
        source
            .find("block=0")
            .ok_or("missing true binding source")?
    );
    let generic = body
        .owners
        .iter()
        .find(|o| o.node_id == expected.id)
        .and_then(|o| o.identifiers.iter().find(|s| s.text == "block"))
        .ok_or("missing earlier default reference")?;
    assert!(generic.source.start_byte < binding.source.start_byte);
    let cases = [
        ("Where are memory blocks decoded?", "decode_after"),
        ("Where are temporary permits checked?", "check"),
        ("Where are blocks rejected?", "decode_after"),
        ("Where are permits allowed?", "check"),
        ("Where are blocks blocks rejected?", "decode_after"),
        ("Where are permits permits allowed?", "check"),
    ];
    let mut answers = Vec::new();
    for mode in [EngineSelection::Json, EngineSelection::Store] {
        let engine = open_with_engine(
            &graph,
            None,
            &d.path().join(format!("cache-{mode:?}")),
            mode,
        )?;
        for (index, (question, target)) in cases.iter().enumerate() {
            let mut query = request();
            query.question = (*question).to_owned();
            let answer = engine.discover_ranked(query)?;
            let candidate = answer
                .candidates
                .iter()
                .find(|c| c.node.name.trim_end_matches("()") == *target)
                .ok_or_else(|| format!("real noun lost: {question}"))?;
            assert_eq!(candidate.relevance, compass_query::RelevanceBand::Low);
            assert!(
                candidate
                    .reasons
                    .iter()
                    .any(|r| r.contains("typed AST-owned parameter binding"))
            );
            assert!(
                !candidate
                    .reasons
                    .iter()
                    .any(|r| r.contains("inspection preference"))
            );
            assert!(
                !answer.candidates.iter().any(|c| matches!(
                    c.node.name.trim_end_matches("()"),
                    "decode_default" | "decode_type" | "enclosing" | "check_default" | "guard"
                )),
                "unproven noun witness: {question}"
            );
            if *target == "decode_after" {
                assert!(candidate.supporting_sources.contains(&binding.source));
            }
            if mode == EngineSelection::Json {
                answers.push(answer.candidates);
            } else {
                assert_eq!(answer.candidates, answers[index]);
            }
        }
    }
    Ok(())
}

#[test]
fn role_coverage_is_query_specific_and_legacy_identifiers_do_not_infer_bindings()
-> Result<(), Box<dyn Error>> {
    use compass_model::source_query::{
        ParameterBindingOmission as R, ParameterBindingStatus as S, SOURCE_QUERY_PROFILE_V5,
    };
    let d = tempfile::tempdir()?;
    let root = d.path().join("repo");
    fs::create_dir(&root)?;
    fs::write(
        root.join("sample.py"),
        "def decode(block):\n    return block\n",
    )?;
    let mut o = options(&root, &d.path().join("out"));
    o.graph_storage = GraphStorage::Sqlite;
    let built = build_local_graph(&o)?;
    let graph = built.output_dir.join("graph.json");
    let sidecar = built.output_dir.join(SOURCE_QUERY_FILE);
    let original = fs::read(&sidecar)?;
    let artifact: SourceQueryArtifact = serde_json::from_slice(&original)?;
    let target = artifact
        .owners
        .first()
        .ok_or("missing source owner")?
        .node_id
        .clone();
    let mut partial = artifact.clone();
    for owner in &mut partial.owners {
        let bindings = owner.parameter_bindings.as_mut().ok_or("missing roles")?;
        bindings.status = S::Partial;
        bindings.reasons.insert(R::TraversalLimit);
    }
    partial.validate()?;
    let mut missing = artifact.clone();
    for owner in &mut missing.owners {
        let bindings = owner.parameter_bindings.as_mut().ok_or("missing roles")?;
        bindings.sites.clear();
        bindings.status = S::Partial;
        bindings.reasons.insert(R::UnsupportedPattern);
    }
    missing.validate()?;
    let mut unsupported = missing.clone();
    for owner in &mut unsupported.owners {
        owner
            .parameter_bindings
            .as_mut()
            .ok_or("missing roles")?
            .status = S::Unsupported;
    }
    unsupported.validate()?;
    let mut legacy = artifact;
    legacy.profile = SOURCE_QUERY_PROFILE_V5.to_owned();
    for owner in &mut legacy.owners {
        owner.parameter_bindings = None;
    }
    legacy.validate()?;
    let variants = [
        (Some(original.clone()), false, false, true),
        (Some(serde_json::to_vec(&partial)?), false, true, true),
        (Some(serde_json::to_vec(&missing)?), false, true, false),
        (Some(serde_json::to_vec(&unsupported)?), false, true, false),
        (Some(serde_json::to_vec(&legacy)?), true, false, false),
        (None, true, false, false),
        (Some(original), false, false, true),
    ];
    for mode in [EngineSelection::Json, EngineSelection::Store] {
        for (bytes, unavailable, limited, retained) in &variants {
            if let Some(bytes) = bytes {
                fs::write(&sidecar, bytes)?;
            } else {
                fs::remove_file(&sidecar)?;
            }
            let engine = open_with_engine(
                &graph,
                None,
                &d.path().join(format!("cache-{mode:?}")),
                mode,
            )?;
            let mut query = request();
            query.question = "Where are memory blocks decoded?".to_owned();
            let answer = engine.discover_ranked(query)?;
            assert_eq!(
                answer.candidates.iter().any(|c| c.node.id == target),
                *retained
            );
            assert_eq!(
                answer.discovery.diagnostics.iter().any(|d| d
                    .message
                    .contains("Typed parameter-binding evidence is unavailable")),
                *unavailable
            );
            assert_eq!(
                answer.discovery.diagnostics.iter().any(|d| d
                    .message
                    .contains("partial or unsupported parameter-binding")),
                *limited
            );
            let mut exact = request();
            exact.question = target.clone();
            let exact = engine.discover_ranked(exact)?;
            assert_eq!(exact.candidates.len(), 1);
            assert_eq!(exact.candidates[0].node.id, target);
            assert!(
                !exact
                    .discovery
                    .diagnostics
                    .iter()
                    .any(|d| d.message.contains("parameter-binding evidence"))
            );
            let mut unrelated = request();
            unrelated.question = "Where is decode implemented?".to_owned();
            let unrelated = engine.discover_ranked(unrelated)?;
            assert!(
                !unrelated
                    .discovery
                    .diagnostics
                    .iter()
                    .any(|d| d.message.contains("parameter-binding evidence"))
            );
            if let Some(bytes) = bytes {
                assert_eq!(
                    fs::read(&sidecar)?,
                    *bytes,
                    "query rewrote immutable evidence"
                );
            } else {
                assert!(!sidecar.exists(), "query invented source evidence");
            }
        }
    }
    Ok(())
}

#[test]
fn nominal_noun_pointers_require_actual_class_or_struct_declarations() -> Result<(), Box<dyn Error>>
{
    use compass_model::code_graph::NodeKind;
    let d = tempfile::tempdir()?;
    let root = d.path().join("repo");
    fs::create_dir(&root)?;
    fs::write(
        root.join("sample.py"),
        concat!(
            "class Block:\n    def decode(self, value):\n        return value\n\n",
            "class Permits:\n    def check(self, value):\n        return value\n\n",
            "def block(value):\n    return value\n\n",
            "def permit(value):\n    return value\n\n",
            "def decode_binding(block):\n    return block\n\n",
            "def check_binding(permits):\n    return permits\n\n",
            "def guard(value):\n    raise ValueError('blocks decoded permits checked rejected allowed')\n",
        ),
    )?;
    fs::write(
        root.join("blocks.py"),
        "def decode_path(value):\n    return value\n",
    )?;
    fs::write(
        root.join("permits.py"),
        "def check_path(value):\n    return value\n",
    )?;
    fs::write(
        root.join("nominal.rs"),
        "pub struct Blocks { pub count: usize }\npub struct Permit { pub count: usize }\n",
    )?;
    let mut o = options(&root, &d.path().join("out"));
    o.graph_storage = GraphStorage::Sqlite;
    let built = build_local_graph(&o)?;
    let graph = built.output_dir.join("graph.json");
    let document = GraphDocument::load(&graph)?;
    let mut baseline = Vec::new();
    for mode in [EngineSelection::Json, EngineSelection::Store] {
        let engine = open_with_engine(
            &graph,
            None,
            &d.path().join(format!("cache-{mode:?}")),
            mode,
        )?;
        for (index, (question, names, binding)) in [
            (
                "Where are memory blocks decoded?",
                ["Block", "Blocks"],
                "decode_binding",
            ),
            (
                "Where are temporary permits checked?",
                ["Permits", "Permit"],
                "check_binding",
            ),
            (
                "Where are blocks rejected?",
                ["Block", "Blocks"],
                "decode_binding",
            ),
            (
                "Where are permits allowed?",
                ["Permits", "Permit"],
                "check_binding",
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let mut query = request();
            query.question = question.to_owned();
            let answer = engine.discover_ranked(query)?;
            for name in names {
                let native = document
                    .nodes
                    .iter()
                    .find(|n| {
                        n.name == name && matches!(n.kind, NodeKind::Class | NodeKind::Struct)
                    })
                    .ok_or_else(|| format!("native nominal missing: {name}"))?;
                let candidate = answer
                    .candidates
                    .iter()
                    .find(|c| c.node.id == native.id)
                    .ok_or_else(|| format!("nominal noun lost: {question} {name}"))?;
                assert_eq!(candidate.relevance, compass_query::RelevanceBand::Low);
                assert!(
                    candidate
                        .reasons
                        .iter()
                        .any(|r| r.contains("lexical type/declaration pointer"))
                );
                assert!(
                    !candidate
                        .reasons
                        .iter()
                        .any(|r| r.contains("typed AST-owned parameter binding")
                            || r.contains("inspection preference"))
                );
                assert!(
                    candidate
                        .supporting_sources
                        .contains(native.source.as_ref().ok_or("nominal source missing")?)
                );
            }
            assert!(
                answer
                    .candidates
                    .iter()
                    .any(|c| c.node.name.trim_end_matches("()") == binding)
            );
            assert!(
                !answer.candidates.iter().any(|c| matches!(
                    c.node.name.trim_end_matches("()"),
                    "block"
                        | "permit"
                        | "guard"
                        | ".decode"
                        | ".check"
                        | "decode_path"
                        | "check_path"
                )),
                "namespace, predicate name or prose borrowed noun: {question}"
            );
            if mode == EngineSelection::Json {
                baseline.push(answer.candidates);
            } else {
                assert_eq!(answer.candidates, baseline[index]);
            }
        }
        for word in ["block", "blocks"] {
            let mut bare = request();
            bare.question = word.to_owned();
            let answer = engine.discover_ranked(bare)?;
            assert!(
                !answer
                    .discovery
                    .diagnostics
                    .iter()
                    .any(|d| d.message.contains("parameter-binding evidence"))
            );
        }
        for name in ["Block", "Permits", "Blocks", "Permit"] {
            let native = document
                .nodes
                .iter()
                .find(|n| n.name == name && matches!(n.kind, NodeKind::Class | NodeKind::Struct))
                .ok_or("missing nominal identity")?;
            let mut exact = request();
            exact.question = native.id.clone();
            let answer = engine.discover_ranked(exact)?;
            assert_eq!(answer.candidates.len(), 1);
            assert_eq!(answer.candidates[0].node.id, native.id);
            assert!(
                !answer.candidates[0]
                    .reasons
                    .iter()
                    .any(|r| r.contains("lexical type/declaration pointer"))
            );
            for question in [name.to_owned(), format!("Where is `{name}`?")] {
                let mut named = request();
                named.question = question;
                let answer = engine.discover_ranked(named)?;
                assert!(
                    answer.candidates.iter().any(|c| c.node.id == native.id),
                    "explicit named identity lost: {name}"
                );
            }
        }
    }
    Ok(())
}

#[test]
fn binding_noun_projection_preserves_subwords_whole_anchors_and_word_boundaries()
-> Result<(), Box<dyn Error>> {
    let d = tempfile::tempdir()?;
    let root = d.path().join("repo");
    fs::create_dir(&root)?;
    fs::write(
        root.join("sample.py"),
        concat!(
            "def decode_camel(blockCount):\n    return blockCount\n\n",
            "def decode_snake(block_count):\n    return block_count\n\n",
            "def decode_plural(blocks):\n    return blocks\n\n",
            "def check_camel(permitCount):\n    return permitCount\n\n",
            "def check_snake(permit_count):\n    return permit_count\n\n",
            "def check_plural(permits):\n    return permits\n\n",
            "def decode_negative(blocker):\n    return blocker\n\n",
            "def check_negative(permitter):\n    return permitter\n",
        ),
    )?;
    fs::write(
        root.join("raw.rs"),
        "fn decode_raw(r#block: usize) -> usize { r#block }\n",
    )?;
    fs::write(
        root.join("dollar.ts"),
        "function decodeDollar($block: number): number { return $block; }\n",
    )?;
    fs::write(
        root.join("grouped.go"),
        "package sample\nfunc decodeGrouped(blockCount, block_count int) int { return blockCount + block_count }\n",
    )?;
    let mut o = options(&root, &d.path().join("out"));
    o.graph_storage = GraphStorage::Sqlite;
    let built = build_local_graph(&o)?;
    let graph = built.output_dir.join("graph.json");
    let document = GraphDocument::load(&graph)?;
    let artifact: SourceQueryArtifact =
        serde_json::from_slice(&fs::read(built.output_dir.join(SOURCE_QUERY_FILE))?)?;
    for mode in [EngineSelection::Json, EngineSelection::Store] {
        let engine = open_with_engine(
            &graph,
            None,
            &d.path().join(format!("cache-{mode:?}")),
            mode,
        )?;
        for (question, spellings) in [
            (
                "Where are memory blocks decoded?",
                vec!["blockCount", "block_count", "blocks", "r#block", "$block"],
            ),
            (
                "Where are temporary permits checked?",
                vec!["permitCount", "permit_count", "permits"],
            ),
        ] {
            let mut query = request();
            query.question = question.to_owned();
            let answer = engine.discover_ranked(query)?;
            for spelling in spellings {
                let owners = artifact
                    .owners
                    .iter()
                    .filter(|o| {
                        o.parameter_bindings
                            .as_ref()
                            .is_some_and(|b| b.sites.iter().any(|s| s.text == spelling))
                    })
                    .collect::<Vec<_>>();
                assert!(!owners.is_empty(), "native binding missing: {spelling}");
                for owner in owners {
                    let binding = owner
                        .parameter_bindings
                        .as_ref()
                        .and_then(|b| b.sites.iter().find(|s| s.text == spelling))
                        .ok_or("missing native binding")?;
                    let candidate = answer
                        .candidates
                        .iter()
                        .find(|c| c.node.id == owner.node_id)
                        .ok_or_else(|| format!("binding noun lost: {question} {spelling}"))?;
                    assert_eq!(candidate.relevance, compass_query::RelevanceBand::Low);
                    assert!(
                        candidate
                            .reasons
                            .iter()
                            .any(|r| r.contains("typed AST-owned parameter binding"))
                    );
                    // A grouped Go owner chooses one deterministic whole-site
                    // witness per noun, rather than attaching every occurrence.
                    let matching = owner
                        .parameter_bindings
                        .as_ref()
                        .ok_or("missing roles")?
                        .sites
                        .iter()
                        .find(|s| {
                            compass_model::search::identifier_search_terms(&s.text)
                                .iter()
                                .any(|t| {
                                    let term = compass_model::canonical_code_token(t.clone());
                                    term == if question.contains("blocks") {
                                        "block"
                                    } else {
                                        "permit"
                                    }
                                })
                        })
                        .ok_or("missing first noun witness")?;
                    assert!(
                        candidate.supporting_sources.contains(&matching.source),
                        "whole binding anchor missing for {} at {}",
                        binding.text,
                        binding.source.start_byte
                    );
                }
            }
            assert!(
                !answer.candidates.iter().any(|c| {
                    document.nodes.iter().any(|n| {
                        n.id == c.node.id
                            && (n.name.contains("negative")
                                || n.qualified_name.contains("negative"))
                    })
                }),
                "word boundary lost: {question}"
            );
        }
    }
    Ok(())
}
