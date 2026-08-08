#![allow(clippy::expect_used)]

use std::path::Path;

use compass_languages::{CandidateRelation, Engine};

#[test]
fn inherited_parameter_properties_resolve_through_this() {
    let source = br#"
abstract class Base {
    constructor(public benchmarks: Record<string, unknown>) {}
    add() { this.benchmarks; }
}
class Child extends Base {
    read() { this.benchmarks; }
}
"#;
    let batch = Engine::default()
        .extract_source_universal_candidate_evidence(
            Path::new("src/inherited.ts"),
            "src/inherited.ts",
            source,
        )
        .expect("candidate evidence");
    let declaration = batch
        .declarations
        .iter()
        .find(|declaration| {
            declaration
                .qualified_name
                .ends_with(".Base.constructor.benchmarks")
        })
        .expect("parameter property declaration");
    assert_eq!(declaration.kind, "parameter");
    let targets = batch
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.relation == CandidateRelation::AccessesMember
                && candidate.target_spelling == "benchmarks"
        })
        .filter_map(|candidate| candidate.constraints.exact_target_declaration_id.as_deref())
        .collect::<Vec<_>>();
    assert_eq!(
        targets,
        vec![declaration.id.as_str(), declaration.id.as_str()]
    );
}
