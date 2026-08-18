#![allow(clippy::expect_used)]

use std::path::Path;

use compass_languages::{CandidateRelation, Engine};

#[test]
fn mapped_and_conditional_type_bindings_are_lexically_scoped() {
    let source = br#"
type Intersection<T> = (T extends any ? (k: T) => void : never) extends (k: infer I) => void ? I : never;
type First<T> = { [K in keyof T]: T[K] };
type Second<T> = { [K in keyof T]?: T[K] };
"#;
    let batch = Engine::default()
        .extract_source_universal_evidence(Path::new("src/types.ts"), "src/types.ts", source)
        .expect("universal evidence");

    for (qualified_suffix, spelling) in [
        (".Intersection.I", "I"),
        (".First.K", "K"),
        (".Second.K", "K"),
    ] {
        let declaration = batch
            .declarations
            .iter()
            .find(|declaration| declaration.qualified_name.ends_with(qualified_suffix))
            .expect("type binding declaration");
        assert!(
            batch.candidates.iter().any(|candidate| {
                candidate.relation == CandidateRelation::References
                    && candidate.target_spelling == spelling
                    && candidate.constraints.exact_target_declaration_id.as_deref()
                        == Some(declaration.id.as_str())
            }),
            "missing exact target for {qualified_suffix}"
        );
    }
}

#[test]
fn inline_object_parameter_members_are_source_anchored() {
    let source = br#"
async function Page(props: { params: Promise<{ slug?: string[] }> }) {
    const params = await props.params;
    return params;
}
"#;
    let batch = Engine::default()
        .extract_source_universal_evidence(Path::new("src/page.tsx"), "src/page.tsx", source)
        .expect("universal evidence");
    let property = batch
        .declarations
        .iter()
        .find(|declaration| declaration.name == "params" && declaration.kind == "property")
        .expect("inline params property");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::AccessesMember
            && candidate.target_spelling == "params"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(property.id.as_str())
    }));
}
