use std::collections::{BTreeSet, HashMap};
use std::error::Error;
use std::path::Path;

use compass_graph::{BuildEvidence, normalize_v1};
use compass_languages::{CandidateRelation, Engine, HierarchyConstraint};
use compass_model::code_graph::EdgeKind;
use compass_resolve::evidence::{
    ResolutionDecision, ResolutionRule, UniversalResolutionIndex, UniversalResolutionLimits,
};
use compass_resolve::resolve;

fn check(
    caller: &str,
    api: &str,
    expected: Option<&str>,
    ambiguous: bool,
) -> Result<(), Box<dyn Error>> {
    let inputs = [
        Engine::default().extract_source(Path::new("src/lib.rs"), caller.as_bytes())?,
        Engine::default().extract_source(Path::new("src/api.rs"), api.as_bytes())?,
    ];
    let batches = inputs
        .iter()
        .map(|input| input.semantic_evidence.clone().ok_or("missing evidence"))
        .collect::<Result<Vec<_>, _>>()?;
    let make = batches[0]
        .declarations
        .iter()
        .find(|declaration| declaration.name == "make")
        .ok_or("missing make")?;
    let calls = batches[0]
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.source_declaration_id == make.id
                && candidate.relation == CandidateRelation::Calls
        })
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 2, "{caller}");
    let index = UniversalResolutionIndex::new(&batches, UniversalResolutionLimits::default())?;
    let mut starts = BTreeSet::new();
    for call in &calls {
        assert!(
            call.constraints.qualified_name.is_none(),
            "per-file method selection: {caller}"
        );
        assert!(matches!(
            call.constraints.hierarchy,
            Some(HierarchyConstraint::RustGenericBounds { .. })
        ));
        let occurrence = batches[0]
            .occurrences
            .iter()
            .find(|occurrence| call.occurrence_id.as_ref() == Some(&occurrence.id))
            .ok_or("missing occurrence")?;
        assert_eq!(
            caller.get(
                usize::try_from(occurrence.range.start_byte)?
                    ..usize::try_from(occurrence.range.end_byte)?
            ),
            Some("value.get")
        );
        assert!(starts.insert(occurrence.range.start_byte));
        let decision = index.resolve(&call.id);
        match expected {
            Some(expected) => {
                let ResolutionDecision::Resolved {
                    declaration_id,
                    evidence,
                } = decision
                else {
                    return Err(format!("expected source target {expected}: {decision:?}").into());
                };
                let target = batches
                    .iter()
                    .flat_map(|batch| &batch.declarations)
                    .find(|declaration| declaration.id == declaration_id)
                    .ok_or("missing target")?;
                assert_eq!(target.qualified_name, expected);
                assert_eq!(evidence.rule, ResolutionRule::RustGenericBoundDispatch);
            }
            None if ambiguous => assert!(
                matches!(decision, ResolutionDecision::Ambiguous { candidate_count } if candidate_count >= 2),
                "{decision:?}: {caller}"
            ),
            None => assert_eq!(decision, ResolutionDecision::Unresolved, "{caller}"),
        }
    }
    let sources = HashMap::from([
        ("src/lib.rs".into(), caller.into()),
        ("src/api.rs".into(), api.into()),
    ]);
    let directory = tempfile::tempdir()?;
    std::fs::create_dir_all(directory.path().join("src"))?;
    for (file, source) in &sources {
        std::fs::write(directory.path().join(file), source)?;
    }
    let normalized = |inputs: &[_]| -> Result<_, Box<dyn Error>> {
        let resolved = resolve(inputs, &sources);
        assert!(resolved.error.is_none(), "{:?}", resolved.error);
        let evidence = BuildEvidence::from_extraction(
            directory.path(),
            &resolved,
            "sha256:generic-bound-dispatch",
        )?;
        Ok(normalize_v1(resolved, evidence)?)
    };
    let graph = normalized(&inputs)?;
    let owner = graph
        .nodes
        .iter()
        .find(|node| node.qualified_name == "crate::make")
        .ok_or("missing graph owner")?;
    let edges = graph
        .links
        .iter()
        .filter(|edge| edge.source == owner.id && edge.kind == EdgeKind::Calls)
        .collect::<Vec<_>>();
    if let Some(expected) = expected {
        assert_eq!(edges.len(), 2);
        let mut actual_sites = BTreeSet::new();
        for edge in edges {
            let target = graph
                .nodes
                .iter()
                .find(|node| node.id == edge.target)
                .ok_or("missing graph target")?;
            assert_eq!(target.qualified_name, expected);
            let site = edge
                .relationship_site
                .as_ref()
                .ok_or("missing source anchor")?;
            assert_eq!(site.file, "src/lib.rs");
            assert!(actual_sites.insert(site.start_byte));
            assert!(
                edge.evidence
                    .iter()
                    .any(|evidence| evidence.anchors.contains(site))
            );
        }
        assert_eq!(actual_sites, starts);
    } else {
        assert!(
            edges.is_empty(),
            "no source call may be published from incomplete/ambiguous bounds"
        );
    }
    let reversed = normalized(&[inputs[1].clone(), inputs[0].clone()])?;
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
fn mixed_bound_order_locality_and_inheritance_preserve_ambiguity() -> Result<(), Box<dyn Error>> {
    for bounds in ["Local + Remote", "Remote + Local"] {
        for (local, remote) in [("Local", "Remote"), ("Remote", "Local")] {
            for api in [
                format!("pub trait {remote} {{ fn get(&self); }}"),
                format!("pub trait Base {{ fn get(&self); }} pub trait {remote}: Base {{}}"),
                format!("pub trait {remote}: Base {{}} pub trait Base {{ fn get(&self); }}"),
            ] {
                let caller = format!(
                    "mod api; use crate::api::{remote}; trait {local} {{ fn get(&self); }} fn make<T: {bounds}>(value: &T) {{ value.get(); value.get(); }}"
                );
                check(&caller, &api, None, true)?;
            }
        }
    }
    check(
        "mod api; use crate::api::Remote; trait Local { fn get(&self); fn get(&self); } fn make<T: Local + Remote>(value: &T) { value.get(); value.get(); }",
        "pub trait Remote { fn get(&self); }",
        None,
        true,
    )
}

#[test]
fn complete_unambiguous_bound_sets_resolve_across_files_and_shared_supertraits()
-> Result<(), Box<dyn Error>> {
    for (caller, api, target) in [
        (
            "mod api; use crate::api::Remote; fn make<T: Remote>(value: &T) { value.get(); value.get(); }",
            "pub trait Remote { fn get(&self); }",
            "crate::api::Remote::get",
        ),
        (
            "mod api; use crate::api::Remote; trait Local {} fn make<T: Local + Remote>(value: &T) { value.get(); value.get(); }",
            "pub trait Remote { fn get(&self); }",
            "crate::api::Remote::get",
        ),
        (
            "mod api; use crate::api::Remote; trait Local {} fn make<T: Remote + Local>(value: &T) { value.get(); value.get(); }",
            "pub trait Remote: Base {} pub trait Base { fn get(&self); }",
            "crate::api::Base::get",
        ),
        (
            "mod api; use crate::api::{Base, Remote}; trait Local: Base {} fn make<T: Local + Remote>(value: &T) { value.get(); value.get(); }",
            "pub trait Base { fn get(&self); } pub trait Remote: Base {}",
            "crate::api::Base::get",
        ),
        (
            "mod api; use crate::api::Exported as Remote; fn make<T: Remote>(value: &T) { value.get(); value.get(); }",
            "mod implementation { pub trait Remote { fn get(&self); } } pub use self::implementation::Remote as Exported;",
            "crate::api::implementation::Remote::get",
        ),
    ] {
        check(caller, api, Some(target), false)?;
    }
    Ok(())
}

#[test]
fn missing_non_trait_and_incomplete_imported_bounds_cannot_disappear() -> Result<(), Box<dyn Error>>
{
    for api in [
        "",
        "pub struct Remote; impl Remote { pub fn get(&self) {} }",
        "pub trait Remote: Missing {}",
        "pub trait Remote { fn get(&self); } pub trait Remote { fn get(&self); }",
    ] {
        let caller = "mod api; use crate::api::Remote; trait Local { fn get(&self); } fn make<T: Local + Remote>(value: &T) { value.get(); value.get(); }";
        // Duplicate trait identities are explicitly ambiguous; missing inventory
        // and a non-trait bound remain unresolved.
        check(
            caller,
            api,
            None,
            api.matches("pub trait Remote").count() == 2,
        )?;
    }
    Ok(())
}

#[test]
fn explicit_trait_qualification_still_disambiguates_competing_bounds() -> Result<(), Box<dyn Error>>
{
    let source = "trait Left { fn get(&self); } trait Right { fn get(&self); } fn make<T: Left + Right>(value: &T) { Left::get(value); Right::get(value); }";
    let input = Engine::default().extract_source(Path::new("src/lib.rs"), source.as_bytes())?;
    let resolved = resolve(
        &[input],
        &HashMap::from([("src/lib.rs".into(), source.into())]),
    );
    assert!(resolved.error.is_none(), "{:?}", resolved.error);
    let owner = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::make")
        .ok_or("missing make")?;
    let mut targets = resolved
        .edges
        .iter()
        .filter(|edge| edge.source == owner.id && edge.string("relation") == "calls")
        .map(|edge| {
            resolved
                .nodes
                .iter()
                .find(|node| node.id == edge.target)
                .map(|node| node.string("qualified_name"))
                .ok_or("missing target")
        })
        .collect::<Result<Vec<_>, _>>()?;
    targets.sort();
    assert_eq!(targets, ["crate::Left::get", "crate::Right::get"]);
    Ok(())
}

#[test]
fn trait_body_macros_cannot_prove_an_empty_competing_member_set() -> Result<(), Box<dyn Error>> {
    for api in [
        "macro_rules! member { () => { fn get(&self); } } pub trait Remote { member!(); }",
        "macro_rules! member { () => { fn get(&self); } } pub trait Remote: Base {} pub trait Base { member!(); }",
        "macro_rules! member { () => { fn get(&self); } } pub trait Remote { fn known(&self); member!(); }",
        "#[transform] pub trait Remote {}",
        "#[doc::transform] pub trait Remote {}",
        "#[transform] mod hidden { pub trait Remote {} } pub use hidden::Remote;",
    ] {
        for bounds in ["Local + Remote", "Remote + Local"] {
            let source = format!(
                "mod api; use crate::api::Remote; trait Local {{ fn get(&self); }} fn make<T: {bounds}>(value: &T) {{ value.get(); value.get(); }}"
            );
            check(&source, api, None, false)?;
        }
    }
    Ok(())
}

#[test]
fn non_type_generic_receiver_annotations_never_emit_invalid_constraints()
-> Result<(), Box<dyn Error>> {
    for source in [
        "fn make<const N: usize>(value: N) { value.get(); }",
        "trait Build { fn get(&self); } fn make<const N: usize>(value: N) where N: Build { value.get(); }",
        "struct Holder<const N: usize> { value: N } impl<const N: usize> Holder<N> { fn make(&self) { self.value.get(); } }",
    ] {
        let input = Engine::default().extract_source(Path::new("src/lib.rs"), source.as_bytes())?;
        let evidence = input.semantic_evidence.as_ref().ok_or("missing evidence")?;
        compass_languages::validate_evidence(
            evidence,
            compass_languages::EvidenceLimits::default(),
        )?;
        assert!(
            !evidence.candidates.iter().any(|candidate| matches!(
                candidate.constraints.hierarchy,
                Some(HierarchyConstraint::RustGenericBounds { .. })
            )),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn const_and_lifetime_parameters_preserve_unrelated_type_bound_dispatch()
-> Result<(), Box<dyn Error>> {
    check(
        "mod api; use crate::api::Remote; fn make<'a, const N: usize, T: Remote>(value: &'a T) { value.get(); value.get(); }",
        "#[allow(dead_code)] pub trait Remote { fn get(&self); }",
        Some("crate::api::Remote::get"),
        false,
    )
}

#[test]
fn missing_and_false_trait_member_metadata_cannot_establish_absence() -> Result<(), Box<dyn Error>>
{
    let inputs = [Engine::default().extract_source(Path::new("src/lib.rs"), b"mod api; use crate::api::Remote; trait Local { fn get(&self); } fn make<T: Local + Remote>(value: &T) { value.get(); }")?, Engine::default().extract_source(Path::new("src/api.rs"), b"pub trait Remote {}")?];
    let batches = inputs
        .iter()
        .map(|input| input.semantic_evidence.clone().ok_or("missing evidence"))
        .collect::<Result<Vec<_>, _>>()?;
    let call = batches[0]
        .candidates
        .iter()
        .find(|candidate| {
            matches!(
                candidate.constraints.hierarchy,
                Some(HierarchyConstraint::RustGenericBounds { .. })
            )
        })
        .ok_or("missing generic call")?
        .id
        .clone();
    assert!(matches!(
        UniversalResolutionIndex::new(&batches, UniversalResolutionLimits::default())?
            .resolve(&call),
        ResolutionDecision::Resolved { .. }
    ));
    for completeness in [None, Some(false)] {
        let mut changed = batches.clone();
        changed[1]
            .declarations
            .iter_mut()
            .find(|declaration| declaration.kind == "trait")
            .ok_or("missing Remote")?
            .member_inventory_complete = completeness;
        let encoded = serde_json::to_value(&changed[1])?;
        if completeness.is_none() {
            assert!(
                encoded["declarations"]
                    .as_array()
                    .ok_or("missing declarations")?
                    .iter()
                    .all(|declaration| declaration.get("memberInventoryComplete").is_none())
            );
        }
        changed[1] = serde_json::from_value(encoded)?;
        assert_eq!(
            UniversalResolutionIndex::new(&changed, UniversalResolutionLimits::default())?
                .resolve(&call),
            ResolutionDecision::Unresolved
        );
    }
    for (language, kind) in [("rust", "struct"), ("python", "trait")] {
        let mut changed = batches[1].clone();
        let declaration = changed
            .declarations
            .iter_mut()
            .find(|declaration| declaration.kind == "trait")
            .ok_or("missing trait")?;
        declaration.language = language.to_owned();
        declaration.kind = kind.to_owned();
        assert_eq!(
            compass_languages::validate_evidence(
                &changed,
                compass_languages::EvidenceLimits::default()
            )
            .err()
            .ok_or("expected invalid metadata")?
            .code,
            compass_languages::EvidenceErrorCode::InvalidFact
        );
    }
    Ok(())
}
