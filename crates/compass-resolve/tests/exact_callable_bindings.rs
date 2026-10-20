//! Source-bound call targets must not escape to an unrelated outer declaration.

use std::collections::{BTreeSet, HashMap};
use std::error::Error;
use std::fs;
use std::path::Path;

use compass_graph::{BuildEvidence, normalize_v1};
use compass_languages::{CandidateRelation, Engine};
use compass_model::code_graph::EdgeKind;
use compass_resolve::evidence::{
    ResolutionDecision, UniversalResolutionIndex, UniversalResolutionLimits,
};

fn check_parameter_calls(path: &str, parameter: &str) -> Result<(), Box<dyn Error>> {
    for outer in ["", "export function callback() {}\n"] {
        let source = format!(
            "{outer}export function invoke({parameter}) {{\n  callback();\n  callback();\n}}\n\
             export class Service {{\n  run() {{}}\n  field = () => {{}};\n\
               use() {{ this.run(); this.field(); }}\n}}\n"
        );
        let extraction = Engine::default().extract_source(Path::new(path), source.as_bytes())?;
        let batch = extraction
            .semantic_evidence
            .as_ref()
            .ok_or("missing universal evidence")?;
        let parameter = batch
            .declarations
            .iter()
            .find(|declaration| declaration.kind == "parameter" && declaration.name == "callback")
            .ok_or("missing callback parameter")?;
        let calls = batch
            .candidates
            .iter()
            .filter(|candidate| {
                candidate.relation == CandidateRelation::Calls
                    && candidate.target_spelling == "callback"
            })
            .collect::<Vec<_>>();
        assert_eq!(
            calls.len(),
            2,
            "two distinct callback occurrences in {path}"
        );
        let index = UniversalResolutionIndex::new(
            std::slice::from_ref(batch),
            UniversalResolutionLimits::default(),
        )
        .map_err(std::io::Error::other)?;
        let mut reordered = batch.clone();
        reordered.declarations.reverse();
        reordered.scopes.reverse();
        reordered.bindings.reverse();
        reordered.occurrences.reverse();
        reordered.candidates.reverse();
        let reordered_index =
            UniversalResolutionIndex::new(&[reordered], UniversalResolutionLimits::default())
                .map_err(std::io::Error::other)?;
        for candidate in &batch.candidates {
            assert_eq!(
                index.resolve(&candidate.id),
                reordered_index.resolve(&candidate.id),
                "fact order cannot select a different target in {path}"
            );
        }
        let mut sites = BTreeSet::new();
        for call in calls {
            assert_eq!(
                call.constraints.exact_target_declaration_id.as_deref(),
                Some(parameter.id.as_str()),
                "the producer already proves the local binding in {path}"
            );
            assert_eq!(
                index.resolve(&call.id),
                ResolutionDecision::Unresolved,
                "the binding is known, but its runtime implementation is unknown in {path}"
            );
            let occurrence = batch
                .occurrences
                .iter()
                .find(|occurrence| Some(&occurrence.id) == call.occurrence_id.as_ref())
                .ok_or("missing call occurrence")?;
            let start = usize::try_from(occurrence.range.start_byte)?;
            let end = usize::try_from(occurrence.range.end_byte)?;
            assert_eq!(source.get(start..end), Some("callback"));
            sites.insert((start, end));
        }
        assert_eq!(
            sites.len(),
            2,
            "repeated calls retain distinct source sites"
        );
        for member in ["run", "field"] {
            let call = batch
                .candidates
                .iter()
                .find(|candidate| {
                    candidate.relation == CandidateRelation::Calls
                        && candidate.target_spelling == member
                })
                .ok_or("missing ordinary member call")?;
            let ResolutionDecision::Resolved {
                declaration_id,
                evidence,
            } = index.resolve(&call.id)
            else {
                return Err(format!("source-proven {member} call unresolved in {path}").into());
            };
            assert_eq!(
                Some(declaration_id.as_str()),
                call.constraints.exact_target_declaration_id.as_deref()
            );
            assert_eq!(
                evidence.rule,
                compass_resolve::evidence::ResolutionRule::ExactSourceDeclaration
            );
        }
        let directory = tempfile::tempdir()?;
        fs::write(directory.path().join(path), &source)?;
        let sources = HashMap::from([(path.to_owned(), source)]);
        let resolved =
            compass_resolve::resolve_with_root(&[extraction], &sources, directory.path());
        assert!(resolved.error.is_none());
        let evidence = BuildEvidence::from_extraction(
            directory.path(),
            &resolved,
            "sha256:exact-callable-bindings",
        )?;
        // Strict publication must succeed without relying on quarantine to
        // remove a fabricated invocation or an invalid parameter endpoint.
        let graph = normalize_v1(resolved, evidence)?;
        let calls = graph
            .links
            .iter()
            .filter(|edge| edge.kind == EdgeKind::Calls)
            .collect::<Vec<_>>();
        assert_eq!(calls.len(), 2, "only the two ordinary member calls remain");
        for edge in calls {
            let caller = graph.nodes.iter().find(|node| node.id == edge.source);
            let target = graph.nodes.iter().find(|node| node.id == edge.target);
            assert!(caller.is_some_and(|node| node.qualified_name.ends_with("Service.use")));
            assert!(target.is_some_and(|node| {
                node.qualified_name.ends_with("Service.run")
                    || node.qualified_name.ends_with("Service.field")
            }));
            let site = edge.relationship_site.as_ref().ok_or("missing call site")?;
            assert_eq!(site.file, path);
            assert!(edge.evidence.iter().any(|provenance| {
                provenance.rule.as_deref() == Some("universal-call-exact-source-declaration")
                    && provenance.anchors.contains(site)
            }));
        }
    }
    Ok(())
}

#[test]
fn typescript_parameter_calls_never_fall_back_to_outer_function() -> Result<(), Box<dyn Error>> {
    check_parameter_calls("main.ts", "callback: () => void")
}

#[test]
fn javascript_parameter_calls_never_fall_back_to_outer_function() -> Result<(), Box<dyn Error>> {
    check_parameter_calls("main.js", "callback")
}

#[test]
fn missing_exact_target_fails_evidence_validation() -> Result<(), Box<dyn Error>> {
    for path in ["main.ts", "main.js"] {
        let source = b"export function callback() {}\nexport function invoke() { callback(); }\n";
        let extraction = Engine::default().extract_source(Path::new(path), source)?;
        let mut batch = extraction
            .semantic_evidence
            .ok_or("missing universal evidence")?;
        let call = batch
            .candidates
            .iter_mut()
            .find(|candidate| candidate.relation == CandidateRelation::Calls)
            .ok_or("missing direct call")?;
        call.constraints.exact_target_declaration_id = Some("unavailable-declaration".to_owned());
        let result = UniversalResolutionIndex::new(&[batch], UniversalResolutionLimits::default());
        assert!(
            matches!(result, Err(error) if error.contains("MissingReference")
                && error.contains("unavailable-declaration")),
            "a missing exact target is invalid evidence in {path}"
        );
    }
    Ok(())
}
