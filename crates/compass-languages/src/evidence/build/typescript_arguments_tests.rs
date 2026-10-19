//! Builder retention controls, independent of project/type resolution.
use super::*;
use crate::{
    TypeScriptTypeArgument as Argument, TypeScriptTypeArguments as Proof, UniversalEvidenceRegistry,
};

fn builder() -> Result<EvidenceBuilder, Box<dyn std::error::Error>> {
    Ok(EvidenceBuilder::new(
        UniversalEvidenceRegistry::pipeline("typescript").ok_or("missing TypeScript pipeline")?,
        "proof-quota-test",
        "main.ts",
        EvidenceLimits::default(),
    ))
}

fn source_builder() -> Result<EvidenceBuilder, Box<dyn std::error::Error>> {
    let mut builder = builder()?;
    builder.batch = crate::Engine::default().extract_source_universal_evidence(
        Path::new("main.ts"), "main.ts",
        b"import type { Holder } from './provider'; class Owner<T> { holder: Holder<T>; read() { this.holder.field.lookup(); } }")?;
    builder.batch.candidates.clear();
    validate_evidence(&builder.batch, EvidenceLimits::default())?;
    Ok(builder)
}

fn holder_id(builder: &EvidenceBuilder) -> Result<String, Box<dyn std::error::Error>> {
    Ok(builder
        .batch
        .bindings
        .iter()
        .find(|binding| binding.spelling == "Holder")
        .ok_or("missing Holder binding")?
        .id
        .clone())
}

fn retain(
    builder: &mut EvidenceBuilder,
    arguments: Option<Proof>,
) -> Result<String, EvidenceError> {
    let site = builder.batch.occurrences.iter().find(|occurrence| {
        occurrence.role == SemanticRole::Call && occurrence.spelling == "lookup"
    });
    let source_id = site.map_or_else(
        || "source".to_owned(),
        |site| site.owner_declaration_id.clone(),
    );
    let site_id = site.map_or_else(|| "site".to_owned(), |site| site.id.clone());
    let binding_id = builder
        .batch
        .bindings
        .iter()
        .find(|binding| binding.spelling == "Holder")
        .map_or_else(|| "binding".to_owned(), |binding| binding.id.clone());
    builder.relate(
        CandidateRelation::Calls,
        &source_id,
        Some(&site_id),
        Some(&binding_id),
        "lookup",
        ResolutionConstraint {
            typescript_type_arguments: arguments,
            ..ResolutionConstraint::default()
        },
    )
}

#[test]
fn repeated_proofs_reserve_the_batch_quota_before_identity_and_retention()
-> Result<(), Box<dyn std::error::Error>> {
    let mut builder = source_builder()?;
    let proof = Proof {
        version: 1,
        owner_binding_id: holder_id(&builder)?,
        arguments: vec![Argument::Unknown; 64],
    };
    retain(&mut builder, Some(proof.clone()))?;
    validate_evidence(&builder.batch, EvidenceLimits::default())?;
    for _ in 1..3125 {
        retain(&mut builder, Some(proof.clone()))?;
    }
    assert_eq!(builder.argument_usage.nodes, 200_000);
    assert_eq!(builder.batch.candidates.len(), 3125);
    let before = builder.argument_usage;
    let error = retain(
        &mut builder,
        Some(Proof {
            arguments: vec![Argument::Unknown],
            ..proof
        }),
    )
    .err()
    .ok_or("extra proof accepted")?;
    assert_eq!(error.code, EvidenceErrorCode::ResourceLimit);
    assert_eq!(builder.argument_usage, before);
    assert_eq!(builder.batch.candidates.len(), 3125);
    assert_eq!(
        builder
            .finish()
            .err()
            .ok_or("partial successful batch")?
            .code,
        EvidenceErrorCode::ResourceLimit
    );
    Ok(())
}

#[test]
fn repeated_proof_bytes_cannot_bypass_retention_accounting()
-> Result<(), Box<dyn std::error::Error>> {
    let mut builder = source_builder()?;
    builder
        .batch
        .bindings
        .iter_mut()
        .find(|binding| binding.spelling == "Holder")
        .ok_or("missing Holder")?
        .id = "b".repeat(1024);
    let proof = Proof {
        version: 1,
        owner_binding_id: "b".repeat(1024),
        arguments: Vec::new(),
    };
    retain(&mut builder, Some(proof.clone()))?;
    validate_evidence(&builder.batch, EvidenceLimits::default())?;
    for _ in 1..16384 {
        retain(&mut builder, Some(proof.clone()))?;
    }
    assert_eq!(builder.argument_usage.bytes, 16 * 1024 * 1024);
    let before = builder.batch.candidates.len();
    assert_eq!(
        retain(&mut builder, Some(proof))
            .err()
            .ok_or("extra text accepted")?
            .code,
        EvidenceErrorCode::ResourceLimit
    );
    assert_eq!(builder.batch.candidates.len(), before);
    assert_eq!(
        builder
            .finish()
            .err()
            .ok_or("partial successful batch")?
            .code,
        EvidenceErrorCode::ResourceLimit
    );
    Ok(())
}

#[test]
fn proof_identity_distinguishes_absence_empty_unknown_and_source_ids()
-> Result<(), Box<dyn std::error::Error>> {
    let mut builder = builder()?;
    let none = retain(&mut builder, None)?;
    let legacy = builder.stable_id(
        "candidate",
        &[
            candidate_relation_name(CandidateRelation::Calls),
            "source",
            "site",
            "binding",
            "lookup",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            "internal_only",
        ],
    );
    assert_eq!(none, legacy);
    let proof = Proof {
        version: 1,
        owner_binding_id: "binding".to_owned(),
        arguments: Vec::new(),
    };
    let empty = retain(&mut builder, Some(proof.clone()))?;
    let unknown = retain(
        &mut builder,
        Some(Proof {
            arguments: vec![Argument::Unknown],
            ..proof.clone()
        }),
    )?;
    let source = retain(
        &mut builder,
        Some(Proof {
            arguments: vec![Argument::Source {
                declaration_id: "one".to_owned(),
                arguments: Vec::new(),
            }],
            ..proof.clone()
        }),
    )?;
    let other = retain(
        &mut builder,
        Some(Proof {
            arguments: vec![Argument::Source {
                declaration_id: "two".to_owned(),
                arguments: Vec::new(),
            }],
            ..proof
        }),
    )?;
    assert_eq!(
        BTreeSet::from([none, empty, unknown, source.clone(), other]).len(),
        5
    );
    assert_eq!(source, retain(&mut builder, builder_proof("one"))?);
    Ok(())
}

fn builder_proof(declaration_id: &str) -> Option<Proof> {
    Some(Proof {
        version: 1,
        owner_binding_id: "binding".to_owned(),
        arguments: vec![Argument::Source {
            declaration_id: declaration_id.to_owned(),
            arguments: Vec::new(),
        }],
    })
}
