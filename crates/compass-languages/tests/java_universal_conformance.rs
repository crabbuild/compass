#![allow(clippy::expect_used)]

use std::path::Path;

use compass_languages::{
    BindingKind, CandidateRelation, Engine, EvidenceLimits, LanguageCapability, SemanticRole,
    UniversalAdapterProfile, validate_evidence,
};

#[test]
fn java_emits_complete_vertical_evidence_without_a_replaced_raw_graph() {
    let source = br#"package org.example.app;

import java.util.List;
import org.example.data.Repository;
import static org.example.util.Helpers.map;
import org.example.model.*;

@Deprecated
public class Service extends BaseService implements Runnable<String, Result>, AutoCloseable {
    private final Repository repository;
    private List<String> names;
    private final Runnable worker = new Runnable() {
        @Override public void run() {}
    };

    public Service(Repository repository) { this.repository = repository; }
    public Service(Repository repository, List<String> names) { this.repository = repository; }

    @Override
    public Result find(String key) {
        Repository local = repository;
        local.load(key);
        map(key);
        return new Result(key);
    }

    public Result find(String key, int limit) { return repository.load(key); }
    public void write(Number number, Appendable appendable) {
        number.toString();
        appendable.append("value");
    }
    public void run() {}
    public void close() {}

    record Nested(String value) {}
}

@interface Audited {}
interface Specialized extends AutoCloseable {}
enum Status {
    READY {
        @Override public String toString() { return "ready"; }
    },
    DONE
}
"#;
    let extraction = Engine::default()
        .extract_source_combined(
            Path::new("/repo/src/main/java/org/example/app/Service.java"),
            "src/main/java/org/example/app/Service.java",
            source,
        )
        .expect("extract Java");
    assert!(extraction.graph.nodes.is_empty());
    assert!(extraction.graph.edges.is_empty());
    assert!(extraction.graph.raw_calls.is_none());

    let evidence = extraction.graph.semantic_evidence.expect("Java evidence");
    validate_evidence(&evidence, EvidenceLimits::default()).expect("valid Java evidence");
    assert_eq!(evidence.adapter.version, 3);
    assert_eq!(
        evidence.adapter.profile,
        UniversalAdapterProfile::UniversalCandidate
    );
    assert!(
        evidence
            .adapter
            .capabilities
            .contains(&LanguageCapability::Namespaces)
    );
    assert!(evidence.declarations.iter().any(|fact| {
        fact.kind == "class"
            && fact.qualified_name == "org.example.app.Service"
            && fact.direct_bases_complete
    }));
    let encoded = serde_json::to_value(&evidence).expect("serialize Java evidence");
    assert!(
        encoded["declarations"]
            .as_array()
            .is_some_and(|declarations| {
                declarations.iter().any(|declaration| {
                    declaration["qualifiedName"] == "org.example.app.Service"
                        && declaration["directBasesComplete"] == true
                })
            })
    );
    assert!(evidence.declarations.iter().any(|fact| {
        fact.kind == "record" && fact.qualified_name == "org.example.app.Service::Nested"
    }));
    assert!(evidence.declarations.iter().any(|fact| {
        fact.kind == "annotation_type" && fact.qualified_name == "org.example.app.Audited"
    }));
    assert!(evidence.declarations.iter().any(|fact| {
        fact.kind == "method" && fact.qualified_name == "org.example.app.Service::worker::run"
    }));
    assert!(
        evidence
            .declarations
            .iter()
            .any(|fact| fact.kind == "enum_member")
    );
    let ready = evidence
        .declarations
        .iter()
        .find(|fact| fact.qualified_name == "org.example.app.Status::READY")
        .expect("READY enum member");
    let ready_to_string = evidence
        .declarations
        .iter()
        .find(|fact| fact.qualified_name == "org.example.app.Status::READY::toString")
        .expect("constant-specific toString method");
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Contains
            && candidate.source_declaration_id == ready.id
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(ready_to_string.id.as_str())
            && candidate.constraints.qualified_name.as_deref()
                == Some(ready_to_string.qualified_name.as_str())
    }));

    let overloads = evidence
        .declarations
        .iter()
        .filter(|fact| fact.name == "find")
        .collect::<Vec<_>>();
    assert_eq!(overloads.len(), 2);
    assert_ne!(overloads[0].graph_node_id, overloads[1].graph_node_id);
    assert_eq!(
        overloads
            .iter()
            .map(|fact| fact.parameter_count)
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from([Some(1), Some(2)])
    );
    assert!(overloads.iter().all(|fact| fact.signature.is_some()));
    assert_eq!(
        overloads
            .iter()
            .map(|fact| fact.parameter_types.clone())
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from([
            vec!["java.lang.String".to_owned()],
            vec!["java.lang.String".to_owned(), "int".to_owned()],
        ])
    );
    assert!(overloads.iter().all(|fact| {
        evidence.candidates.iter().any(|candidate| {
            candidate.relation == CandidateRelation::Contains
                && candidate.constraints.exact_target_declaration_id.as_deref()
                    == Some(fact.id.as_str())
        })
    }));

    assert!(evidence.bindings.iter().any(|binding| {
        binding.spelling == "Repository"
            && binding.qualified_target == "org.example.data.Repository"
    }));
    assert!(evidence.bindings.iter().any(|binding| {
        binding.spelling == "map"
            && binding.qualified_target == "org.example.util.Helpers.map"
            && binding.kind == BindingKind::ImportAlias
    }));
    assert!(evidence.bindings.iter().any(|binding| {
        binding.spelling == "*" && binding.qualified_target == "org.example.model.*"
    }));
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Extends
            && candidate.target_spelling == "BaseService"
    }));
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Extends
            && candidate.target_spelling == "AutoCloseable"
            && evidence.declarations.iter().any(|declaration| {
                declaration.id == candidate.source_declaration_id
                    && declaration.qualified_name == "org.example.app.Specialized"
            })
    }));
    assert_eq!(
        evidence
            .candidates
            .iter()
            .filter(|candidate| candidate.relation == CandidateRelation::Implements)
            .map(|candidate| candidate.target_spelling.as_str())
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from(["AutoCloseable", "Runnable"])
    );
    assert!(evidence.occurrences.iter().any(|occurrence| {
        occurrence.role == SemanticRole::Annotation && occurrence.spelling == "Override"
    }));
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.target_spelling == "load"
            && candidate.constraints.qualified_name.as_deref()
                == Some("org.example.data.Repository::load")
            && candidate.constraints.argument_count == Some(1)
            && candidate.constraints.argument_types == [Some("java.lang.String".to_owned())]
    }));
    for qualified_name in ["java.lang.Number::toString", "java.lang.Appendable::append"] {
        assert!(evidence.candidates.iter().any(|candidate| {
            candidate.relation == CandidateRelation::Calls
                && candidate.constraints.qualified_name.as_deref() == Some(qualified_name)
        }));
    }
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Constructs
            && candidate.target_spelling == "Result"
            && candidate.constraints.argument_count == Some(1)
    }));
}

#[test]
fn java_evidence_is_deterministic_and_source_bounded_under_parser_recovery() {
    let source = "package café.example; class Unicode { void naïve() { helper(\"é\"); }";
    let first = Engine::default()
        .extract_source(Path::new("src/Unicode.java"), source.as_bytes())
        .expect("first extraction")
        .semantic_evidence
        .expect("first evidence");
    let second = Engine::default()
        .extract_source(Path::new("src/Unicode.java"), source.as_bytes())
        .expect("second extraction")
        .semantic_evidence
        .expect("second evidence");
    assert_eq!(first, second);
    assert!(
        first
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code == "partial_parser_recovery" })
    );
    assert!(first.occurrences.iter().all(|occurrence| {
        usize::try_from(occurrence.range.end_byte).expect("bounded offset") <= source.len()
    }));
    assert!(
        first
            .declarations
            .iter()
            .filter(|declaration| declaration.kind == "class")
            .all(|declaration| !declaration.direct_bases_complete)
    );
}
