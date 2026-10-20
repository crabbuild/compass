use std::error::Error;
use std::path::Path;

use compass_languages::{CandidateRelation, Engine};
use serde::Deserialize;

#[derive(Deserialize)]
struct Expected {
    kind: String,
    line: u32,
}

#[derive(Deserialize)]
struct Case {
    prefix: String,
    kind: String,
    owner: String,
    target: String,
    relation: String,
    expected: Option<Expected>,
    typescript: String,
    javascript: String,
}

fn check_cases(ambiguous: bool) -> Result<(), Box<dyn Error>> {
    let cases: Vec<Case> = serde_json::from_str(include_str!(
        "../../../fixtures/typescript_member_scopes.json"
    ))?;
    let mut engine = Engine::default();
    for case in cases {
        if (case.kind == "ambiguous") != ambiguous {
            continue;
        }
        for (extension, source) in [("ts", &case.typescript), ("js", &case.javascript)] {
            for seed in 0..24 {
                let file = format!("{}{:02}.{extension}", case.prefix, seed);
                let batch = engine.extract_source_universal_evidence(
                    Path::new(&file),
                    &file,
                    source.as_bytes(),
                )?;
                let relation = match case.relation.as_str() {
                    "calls" => CandidateRelation::Calls,
                    "accesses_member" => CandidateRelation::AccessesMember,
                    _ => return Err("unsupported fixture relation".into()),
                };
                let candidates = batch
                    .candidates
                    .iter()
                    .filter(|candidate| {
                        candidate.relation == relation
                            && candidate.target_spelling == case.target
                            && batch.declarations.iter().any(|declaration| {
                                declaration.id == candidate.source_declaration_id
                                    && declaration.name == case.owner
                            })
                    })
                    .collect::<Vec<_>>();
                assert_eq!(candidates.len(), 1, "{file}");
                let candidate = candidates[0];
                if let Some(expected) = &case.expected {
                    let target = candidate
                        .constraints
                        .exact_target_declaration_id
                        .as_ref()
                        .and_then(|id| batch.declarations.iter().find(|d| &d.id == id))
                        .ok_or_else(|| format!("{file}: missing proven member target"))?;
                    assert_eq!(target.name, case.target, "{file}");
                    assert_eq!(target.kind, expected.kind, "{file}");
                    assert_eq!(target.range.start_line, expected.line, "{file}");
                } else {
                    assert!(
                        candidate.constraints.exact_target_declaration_id.is_none(),
                        "{file}: a hash-selected child scope must not prove a target"
                    );
                }
            }
        }
    }
    Ok(())
}

#[test]
fn ambiguous_accessor_targets_do_not_depend_on_filename_hashes() -> Result<(), Box<dyn Error>> {
    check_cases(true)
}

#[test]
fn primary_scope_preserves_members_nested_calls_and_shadowing() -> Result<(), Box<dyn Error>> {
    check_cases(false)
}

#[test]
fn javascript_field_declarations_keep_the_property_name_anchor() -> Result<(), Box<dyn Error>> {
    let source = "class Owner { declared = 1; callback = () => {}; #hidden = 2; }";
    let batch = Engine::default().extract_source_universal_evidence(
        Path::new("fields.js"),
        "fields.js",
        source.as_bytes(),
    )?;
    for name in ["declared", "callback", "#hidden"] {
        let fields = batch
            .declarations
            .iter()
            .filter(|declaration| declaration.kind == "property" && declaration.name == name)
            .collect::<Vec<_>>();
        assert_eq!(fields.len(), 1, "missing or duplicated field {name}");
        let range = &fields[0].range;
        assert_eq!(range.source_file, "fields.js");
        assert_eq!(
            source.get(usize::try_from(range.start_byte)?..usize::try_from(range.end_byte)?),
            Some(name)
        );
    }
    Ok(())
}
