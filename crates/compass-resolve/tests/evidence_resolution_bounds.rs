use std::error::Error;
use std::path::Path;

use compass_languages::{CandidateRelation, Engine};
use compass_resolve::evidence::{
    ResolutionDecision, UniversalResolutionIndex, UniversalResolutionLimits,
};

#[test]
fn rust_outer_return_overflow_remains_ambiguous() -> Result<(), Box<dyn Error>> {
    let source = br#"
struct Alpha;
struct Beta;
impl Alpha { fn finish(&self) {} }
impl Beta { fn finish(&self) {} }
struct Factory;
impl Factory { fn make() -> Alpha { Alpha } }
fn caller() { Factory::make().finish(); }
"#;
    let extraction = Engine::default().extract_source(Path::new("src/lib.rs"), source)?;
    let mut evidence = extraction
        .semantic_evidence
        .ok_or("missing Rust semantic evidence")?;

    let outer_occurrence_id = evidence
        .occurrences
        .iter()
        .find(|occurrence| occurrence.context.as_deref() == Some("rust-outer-nominal-return"))
        .map(|occurrence| occurrence.id.clone())
        .ok_or("missing outer nominal return occurrence")?;
    let mut competing_return = evidence
        .candidates
        .iter()
        .find(|candidate| candidate.occurrence_id.as_deref() == Some(&outer_occurrence_id))
        .cloned()
        .ok_or("missing outer nominal return candidate")?;
    let beta = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "crate::Beta")
        .ok_or("missing Beta declaration")?;
    competing_return.id.push_str(":competing");
    competing_return.target_spelling = "Beta".to_owned();
    competing_return.constraints.exact_target_declaration_id = Some(beta.id.clone());
    competing_return.constraints.qualified_name = Some(beta.qualified_name.clone());
    evidence.candidates.push(competing_return);

    let member_candidate = evidence
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == CandidateRelation::Calls && candidate.target_spelling == "finish"
        })
        .map(|candidate| candidate.id.clone())
        .ok_or("missing chained finish candidate")?;
    let index = UniversalResolutionIndex::new(
        &[evidence],
        UniversalResolutionLimits {
            candidates_per_lookup: 1,
            ..UniversalResolutionLimits::default()
        },
    )?;

    assert_eq!(
        index.resolve(&member_candidate),
        ResolutionDecision::Ambiguous { candidate_count: 2 }
    );
    Ok(())
}
