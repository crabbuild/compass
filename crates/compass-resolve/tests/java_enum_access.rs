use std::collections::{BTreeSet, HashMap};
use std::error::Error;
use std::path::Path;

use compass_graph::{BuildEvidence, normalize_v1};
use compass_languages::Engine;
use compass_model::code_graph::{EdgeKind, GraphDocument, NodeKind};
use compass_resolve::resolve;

fn publish(
    root: &Path,
    files: &[(&str, &str)],
    reverse: bool,
) -> Result<GraphDocument, Box<dyn Error>> {
    let mut sources = HashMap::new();
    let mut inputs = Vec::new();
    for &(file, source) in files {
        let path = root.join(file);
        std::fs::create_dir_all(path.parent().ok_or("missing parent")?)?;
        std::fs::write(path, source)?;
        sources.insert(file.to_owned(), source.to_owned());
        inputs.push(Engine::default().extract_source(Path::new(file), source.as_bytes())?);
    }
    if reverse {
        inputs.reverse();
    }
    let resolved = resolve(&inputs, &sources);
    assert!(resolved.error.is_none(), "{:?}", resolved.error);
    let evidence = BuildEvidence::from_extraction(root, &resolved, "sha256:java-enum-access")?;
    Ok(normalize_v1(resolved, evidence)?)
}

fn member_pairs(graph: &GraphDocument) -> Result<Vec<(String, String)>, Box<dyn Error>> {
    let mut result = Vec::new();
    for edge in &graph.links {
        if !edge
            .occurrence_rule
            .as_ref()
            .is_some_and(|rule| rule.as_str().starts_with("universal-member-access-"))
        {
            continue;
        }
        assert_eq!(edge.kind, EdgeKind::References);
        let source = graph
            .nodes
            .iter()
            .find(|n| n.id == edge.source)
            .ok_or("source")?;
        let target = graph
            .nodes
            .iter()
            .find(|n| n.id == edge.target)
            .ok_or("target")?;
        let site = edge.relationship_site.as_ref().ok_or("member site")?;
        assert!(
            edge.evidence
                .iter()
                .any(|evidence| evidence.anchors.contains(site))
        );
        result.push((source.qualified_name.clone(), target.qualified_name.clone()));
    }
    result.sort();
    Ok(result)
}

#[test]
fn cross_file_enum_references_keep_direction_multiplicity_provenance_and_order()
-> Result<(), Box<dyn Error>> {
    let model = "package model; public enum State { READY, DONE }";
    let caller = "package app; // λ\nimport model.State;\nclass Box {\n State run() {\n  return State.READY == State.DONE ? State.READY : State.DONE;\n }\n}\n";
    let files = [("model/State.java", model), ("app/Box.java", caller)];
    let root = tempfile::tempdir()?;
    let graph = publish(root.path(), &files, false)?;
    let run = graph
        .nodes
        .iter()
        .find(|n| n.qualified_name == "app.Box::run")
        .ok_or("run")?;
    let mut starts = BTreeSet::new();
    for edge in graph.links.iter().filter(|edge| edge.source == run.id) {
        let target = graph
            .nodes
            .iter()
            .find(|n| n.id == edge.target)
            .ok_or("target")?;
        if target.kind != NodeKind::EnumMember {
            continue;
        }
        assert_eq!(edge.kind, EdgeKind::References);
        assert!(
            edge.occurrence_rule
                .as_ref()
                .is_some_and(|rule| { rule.as_str().starts_with("universal-member-access-") })
        );
        let site = edge.relationship_site.as_ref().ok_or("site")?;
        assert_eq!(site.file, "app/Box.java");
        assert_eq!(site.start_line, 5);
        let start = usize::try_from(site.start_byte)?;
        let end = usize::try_from(site.end_byte)?;
        assert_eq!(caller.get(start..end), Some(target.name.as_str()));
        assert!(starts.insert(start));
        assert!(
            edge.evidence
                .iter()
                .any(|evidence| evidence.anchors.contains(site))
        );
    }
    assert_eq!(starts.len(), 4, "{:#?}", graph.links);
    let reversed = publish(root.path(), &files, true)?;
    assert_eq!(
        serde_json::to_value(&graph.nodes)?,
        serde_json::to_value(&reversed.nodes)?
    );
    assert_eq!(
        serde_json::to_value(&graph.links)?,
        serde_json::to_value(&reversed.links)?
    );
    Ok(())
}

#[test]
fn constant_specific_bodies_distinguish_own_fields_from_enclosing_enum_values()
-> Result<(), Box<dyn Error>> {
    let source = "package p; enum State { A { int B = 1; int size() { return B + this.B; } State next() { return State.B; } }, B; }";
    let root = tempfile::tempdir()?;
    let graph = publish(root.path(), &[("p/State.java", source)], false)?;
    assert_eq!(
        member_pairs(&graph)?,
        [
            ("p.State::A::next".into(), "p.State::B".into()),
            ("p.State::A::size".into(), "p.State::A::B".into()),
            ("p.State::A::size".into(), "p.State::A::B".into()),
        ]
    );
    Ok(())
}

#[test]
fn enum_switch_labels_use_the_cross_file_selector_type() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let graph = publish(
        root.path(),
        &[
            (
                "model/State.java",
                "package model; public enum State { READY, DONE }",
            ),
            (
                "app/Box.java",
                "package app; import model.State; class Box { static final int READY = 1; int run(State state) { switch (state) { case READY: return 1; case DONE: return 2; default: return 0; } } }",
            ),
        ],
        false,
    )?;
    assert_eq!(
        member_pairs(&graph)?,
        [
            ("app.Box::run".into(), "model.State::DONE".into()),
            ("app.Box::run".into(), "model.State::READY".into()),
        ]
    );
    Ok(())
}

#[test]
fn duplicate_receiver_types_are_not_disambiguated_by_enum_member_availability()
-> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let files = [
        ("one/State.java", "package p; enum State { READY }"),
        ("two/State.java", "package p; enum State { DONE }"),
        (
            "p/Box.java",
            "package p; class Box { State run(State state) { switch (state) { case READY: return State.READY; default: return state; } } }",
        ),
    ];
    for reverse in [false, true] {
        assert!(member_pairs(&publish(root.path(), &files, reverse)?)?.is_empty());
    }
    Ok(())
}

#[test]
fn shadowing_unknown_switches_and_unregistered_owners_do_not_invent_enum_edges()
-> Result<(), Box<dyn Error>> {
    for source in [
        "package p; enum State { READY } class Box { Object run(Object State) { return State.READY; } }",
        "package p; enum State { READY } class Box { <State> Object run() { return State.READY; } }",
        "package p; enum State { READY } class Box { void run() { class State {} sink(State.READY); } }",
        "package p; enum State { READY } class Box { int READY; int run() { switch (unknown()) { case READY: return 0; default: return 1; } } }",
        "package p; enum State { READY } class Box { int run(Unknown state) { switch (state) { case READY: return 0; default: return 1; } } }",
        "package p; enum State { READY; void run() { Object value = new Object() { State get() { return READY; } }; } }",
    ] {
        let root = tempfile::tempdir()?;
        assert!(
            member_pairs(&publish(root.path(), &[("p/State.java", source)], false)?)?.is_empty(),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn enum_values_do_not_expose_constant_body_fields_to_other_classes() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let graph = publish(
        root.path(),
        &[
            (
                "model/State.java",
                "package model; public enum State { READY { int hidden = 1; } }",
            ),
            (
                "app/Box.java",
                "package app; import static model.State.READY; class Box { int run() { return READY.hidden; } }",
            ),
        ],
        false,
    )?;
    assert!(member_pairs(&graph)?.is_empty());
    Ok(())
}

#[test]
fn same_package_enum_type_and_field_initializer_references_resolve() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let graph = publish(
        root.path(),
        &[
            (
                "p/State.java",
                "package p; public enum State { READY { int value = 1; int next = value; }, DONE; }",
            ),
            (
                "p/Box.java",
                "package p; class Box { State run() { return State.READY; } }",
            ),
        ],
        false,
    )?;
    assert_eq!(
        member_pairs(&graph)?,
        [
            ("p.Box::run".into(), "p.State::READY".into()),
            (
                "p.State::READY::next".into(),
                "p.State::READY::value".into()
            ),
        ]
    );
    Ok(())
}

#[test]
fn enum_body_field_resolution_requires_bounded_lexical_ownership() -> Result<(), Box<dyn Error>> {
    use compass_languages::CandidateRelation;
    use compass_resolve::evidence::{
        ResolutionDecision, UniversalResolutionIndex, UniversalResolutionLimits,
    };
    let source = "package p; enum State { A { int value; int run() { return value; } }; }";
    let evidence = Engine::default()
        .extract_source(Path::new("State.java"), source.as_bytes())?
        .semantic_evidence
        .ok_or("missing Java evidence")?;
    let candidate = evidence
        .candidates
        .iter()
        .find(|candidate| candidate.relation == CandidateRelation::AccessesMember)
        .ok_or("missing access")?
        .clone();
    let target = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "p.State::A::value")
        .ok_or("field")?
        .id
        .clone();
    for budget in [1, 256] {
        let index = UniversalResolutionIndex::new(
            std::slice::from_ref(&evidence),
            UniversalResolutionLimits {
                candidates_per_lookup: budget,
                ..UniversalResolutionLimits::default()
            },
        )?;
        let decision = index.resolve(&candidate.id);
        if budget == 1 {
            assert_eq!(decision, ResolutionDecision::Unresolved);
        } else {
            assert!(
                matches!(decision, ResolutionDecision::Resolved { declaration_id, .. } if declaration_id == target)
            );
        }
    }
    let module_scope = evidence
        .scopes
        .iter()
        .find(|scope| scope.kind == "module")
        .ok_or("missing module scope")?
        .id
        .clone();
    for scope in [None, Some(module_scope)] {
        let mut foreign = evidence.clone();
        let access = foreign
            .candidates
            .iter_mut()
            .find(|c| c.id == candidate.id)
            .ok_or("candidate")?;
        access.constraints.scope_id = scope;
        let index =
            UniversalResolutionIndex::new(&[foreign], UniversalResolutionLimits::default())?;
        assert_eq!(index.resolve(&candidate.id), ResolutionDecision::Unresolved);
    }
    Ok(())
}

#[test]
fn nested_enum_type_shadows_import_before_member_selection() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let graph = publish(
        root.path(),
        &[
            (
                "remote/State.java",
                "package remote; public enum State { A }",
            ),
            (
                "p/Box.java",
                "package p; import remote.State; class Box { enum State { A } State run() { return State.A; } }",
            ),
        ],
        false,
    )?;
    assert_eq!(
        member_pairs(&graph)?,
        [("p.Box::run".into(), "p.Box::State::A".into())]
    );
    Ok(())
}

#[test]
fn imported_nested_enum_nominal_receivers_keep_exact_occurrences() -> Result<(), Box<dyn Error>> {
    let model = "package model; public class Outer { public static class Middle { public enum State { READY, DONE; public int code; } } }";
    let caller = "package app; // λ\nimport model.Outer.Middle.State;\nclass Box {\n State direct() { return State.READY; }\n State viaValue(State value) { return value.DONE; }\n int code(State value) { return value.code; }\n int choose(State value) { switch (value) { case READY: return 1; default: return 0; } }\n Object shadow(Object State) { return State.READY; }\n <State> Object typeParameter() { return State.READY; }\n}\n";
    let files = [("model/Outer.java", model), ("app/Box.java", caller)];
    let root = tempfile::tempdir()?;
    let graph = publish(root.path(), &files, false)?;
    assert_eq!(
        member_pairs(&graph)?,
        [
            (
                "app.Box::choose".into(),
                "model.Outer::Middle::State::READY".into()
            ),
            (
                "app.Box::code".into(),
                "model.Outer::Middle::State::code".into()
            ),
            (
                "app.Box::direct".into(),
                "model.Outer::Middle::State::READY".into()
            ),
            (
                "app.Box::viaValue".into(),
                "model.Outer::Middle::State::DONE".into()
            ),
        ]
    );
    for edge in graph.links.iter().filter(|edge| {
        edge.occurrence_rule
            .as_ref()
            .is_some_and(|rule| rule.as_str().starts_with("universal-member-access-"))
    }) {
        let site = edge.relationship_site.as_ref().ok_or("site")?;
        assert_eq!(site.file, "app/Box.java");
        let token = caller
            .get(usize::try_from(site.start_byte)?..usize::try_from(site.end_byte)?)
            .ok_or("token")?;
        assert!(matches!(token, "READY" | "DONE" | "code"));
    }
    let reversed = publish(root.path(), &files, true)?;
    assert_eq!(
        serde_json::to_value(&graph.nodes)?,
        serde_json::to_value(&reversed.nodes)?
    );
    assert_eq!(
        serde_json::to_value(&graph.links)?,
        serde_json::to_value(&reversed.links)?
    );
    Ok(())
}

#[test]
fn dotted_nested_receiver_ambiguity_is_not_resolved_by_member_availability()
-> Result<(), Box<dyn Error>> {
    for extra in [
        (
            "duplicate/EmptyOuter.java",
            "package model; public class Outer {}",
        ),
        (
            "duplicate/Outer.java",
            "package model; public class Outer { public enum State { OTHER } }",
        ),
        (
            "model/Outer/State.java",
            "package model.Outer; public enum State { OTHER }",
        ),
    ] {
        let root = tempfile::tempdir()?;
        let graph = publish(
            root.path(),
            &[
                (
                    "model/Outer.java",
                    "package model; public class Outer { public enum State { READY } }",
                ),
                extra,
                (
                    "app/Box.java",
                    "package app; import model.Outer.State; class Box { State run() { return State.READY; } }",
                ),
            ],
            false,
        )?;
        assert!(member_pairs(&graph)?.is_empty());
    }
    Ok(())
}

#[test]
fn dotted_nominal_decisions_preserve_limits_kinds_and_exact_precedence()
-> Result<(), Box<dyn Error>> {
    use compass_languages::{
        CandidateRelation, EvidenceBuilder, EvidenceLimits, EvidenceRange, ResolutionConstraint,
        SemanticRole, UniversalEvidenceRegistry,
    };
    use compass_resolve::evidence::{
        ResolutionDecision, UniversalResolutionIndex, UniversalResolutionLimits,
    };
    let pipeline = UniversalEvidenceRegistry::pipeline("java").ok_or("Java pipeline")?;
    // Lowercase source type names deliberately forbid a capitalization heuristic.
    for (parents, owners, collision, budget, exact, kind_allowed, resolves) in [
        (1, 1, false, 256, false, true, true),
        (0, 1, false, 256, false, true, false),
        (2, 1, false, 256, false, true, false),
        (1, 2, false, 256, false, true, false),
        (1, 1, true, 256, false, true, false),
        (1, 2, false, 1, false, true, false),
        (1, 1, false, 1, false, true, false),
        (1, 1, false, 2, false, true, false),
        (1, 1, false, 3, false, true, true),
        (0, 2, true, 1, true, true, true),
        (1, 1, false, 256, false, false, false),
    ] {
        let range = |start: u32| EvidenceRange {
            source_file: "fixture.java".into(),
            start_byte: u64::from(start),
            end_byte: u64::from(start + 1),
            start_line: 1,
            end_line: 1,
            start_column: start,
            end_column: start + 1,
        };
        let mut builder = EvidenceBuilder::new(
            pipeline,
            "dotted-owner",
            "fixture.java",
            EvidenceLimits::default(),
        );
        let caller = builder.declare(
            "method",
            "caller",
            "run",
            "p.Box::run",
            Some("p"),
            None,
            range(0),
        )?;
        let field = builder.declare(
            "field",
            "field",
            "value",
            "p.outer::middle::state::value",
            Some("p"),
            None,
            range(2),
        )?;
        for i in 0..parents {
            builder.declare(
                "class",
                &format!("outer-{i}"),
                "outer",
                "p.outer",
                Some("p"),
                None,
                range(4 + i),
            )?;
        }
        builder.declare(
            "class",
            "middle",
            "middle",
            "p.outer::middle",
            Some("p"),
            None,
            range(8),
        )?;
        for i in 0..owners {
            builder.declare(
                "class",
                &format!("state-{i}"),
                "state",
                "p.outer::middle::state",
                Some("p"),
                None,
                range(10 + i),
            )?;
        }
        if collision {
            builder.declare(
                "class",
                "package-collision",
                "state",
                "p.outer.middle.state",
                Some("p.outer.middle"),
                None,
                range(15),
            )?;
        }
        let occurrence = builder.occur(
            SemanticRole::MemberAccess,
            &caller,
            "value",
            Some("item"),
            None,
            range(20),
        )?;
        let candidate = builder.relate(
            CandidateRelation::AccessesMember,
            &caller,
            Some(&occurrence),
            None,
            "value",
            ResolutionConstraint {
                qualified_name: Some("p.outer.middle.state::value".into()),
                allowed_target_kinds: vec![
                    if kind_allowed { "field" } else { "enum_member" }.into(),
                ],
                exact_target_declaration_id: exact.then(|| field.clone()),
                ..ResolutionConstraint::default()
            },
        )?;
        let index = UniversalResolutionIndex::new(
            &[builder.finish()?],
            UniversalResolutionLimits {
                candidates_per_lookup: budget,
                ..UniversalResolutionLimits::default()
            },
        )?;
        let decision = index.resolve(&candidate);
        if resolves {
            assert!(
                matches!(decision, ResolutionDecision::Resolved { declaration_id, .. } if declaration_id == field)
            );
        } else if owners > 1 || collision || parents > 1 {
            assert!(matches!(
                decision,
                ResolutionDecision::Ambiguous { candidate_count: 2 }
            ));
        } else {
            assert_eq!(decision, ResolutionDecision::Unresolved);
        }
    }
    Ok(())
}
