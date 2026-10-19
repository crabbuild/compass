use std::error::Error;
use std::path::Path;

use compass_languages::{
    CandidateRelation, Engine, EvidenceErrorCode, EvidenceLimits, SemanticEvidenceBatch,
    SymbolNamespace, TypeScriptTypeArgument as Argument, validate_evidence,
};

fn batch() -> Result<SemanticEvidenceBatch, Box<dyn Error>> {
    let source = br#"import type { Holder, Remote } from './provider';
class Good { lookup() {} }
class Owner<T> {
    known: Holder<Good>;
    unknown: Holder<T>;
    nested: Holder<Remote<Good>>;
    read() { this.known.field.lookup(); this.unknown.field.lookup(); this.nested.field.lookup(); }
}"#;
    Ok(Engine::default().extract_source_universal_evidence(
        Path::new("main.ts"),
        "main.ts",
        source,
    )?)
}

#[test]
fn class_inventory_certifies_retained_syntax_and_separate_base_coverage()
-> Result<(), Box<dyn Error>> {
    for (source, expected) in [
        (
            "import type { X } from './missing'; class Good { lookup() {} }",
            true,
        ),
        (
            "import type { X } from './missing'; class Good { then(done: () => void) {} lookup() {} }",
            true,
        ),
        ("class Good { lookup() {} }", false),
        (
            "import type { X } from './missing'; class Good extends X { lookup() {} }",
            false,
        ),
        (
            "import type { X } from './missing'; class Good { constructor(public value: X) {} lookup() {} }",
            false,
        ),
        (
            "import type { X } from './missing'; class Good { ['then']() {} lookup() {} }",
            false,
        ),
        (
            r"import type { X } from './missing'; class Good { \u0074hen(done: () => void) {} lookup() {} }",
            false,
        ),
        (
            "import type { X } from './missing'; @decorate class Good { lookup() {} }",
            false,
        ),
        (
            "import type { X } from './missing'; namespace Scope { class Good { lookup() {} } }",
            false,
        ),
        (
            "import type { X } from './missing'; declare class Good { lookup(): void; }",
            false,
        ),
    ] {
        let batch = Engine::default().extract_source_universal_evidence(
            Path::new("main.ts"),
            "main.ts",
            source.as_bytes(),
        )?;
        validate_evidence(&batch, EvidenceLimits::default())?;
        let class = batch
            .declarations
            .iter()
            .find(|fact| fact.kind == "class" && fact.name == "Good")
            .ok_or("missing Good class")?;
        assert_eq!(class.member_inventory_complete, Some(expected), "{source}");
        assert_eq!(class.direct_bases_complete, expected, "{source}");
        let wire = serde_json::to_vec(&batch)?;
        assert_eq!(
            serde_json::from_slice::<SemanticEvidenceBatch>(&wire)?,
            batch
        );
    }
    Ok(())
}

#[test]
fn merge_inventory_is_explicit_and_rejects_missing_or_ambient_syntax() -> Result<(), Box<dyn Error>>
{
    for (source, expected) in [
        ("", true),
        ("// empty input", true),
        ("export interface Item { inspect(): void }", true),
        ("interface Item { inspect(): void }", false),
        (
            "import './types'; declare module './types' { interface Item { then(done: () => void): void } }",
            false,
        ),
        (
            "export {}; declare global { interface Object { then(done: () => void): void } }",
            false,
        ),
    ] {
        let batch = Engine::default().extract_source_universal_evidence(
            Path::new("main.ts"),
            "main.ts",
            source.as_bytes(),
        )?;
        assert_eq!(
            batch.typescript_merge_inventory_complete,
            Some(expected),
            "{source}"
        );
        validate_evidence(&batch, EvidenceLimits::default())?;
        let mut legacy = serde_json::to_value(&batch)?;
        legacy
            .as_object_mut()
            .ok_or("missing batch object")?
            .remove("typescriptMergeInventoryComplete");
        let legacy: SemanticEvidenceBatch = serde_json::from_value(legacy)?;
        assert_eq!(legacy.typescript_merge_inventory_complete, None);
    }
    Ok(())
}

#[test]
fn class_inventory_metadata_is_not_valid_for_arbitrary_declarations() -> Result<(), Box<dyn Error>>
{
    let mut batch = batch()?;
    let fact = batch
        .declarations
        .iter_mut()
        .find(|fact| fact.kind == "method")
        .ok_or("missing method")?;
    fact.member_inventory_complete = Some(true);
    assert!(validate_evidence(&batch, EvidenceLimits::default()).is_err());
    Ok(())
}

#[test]
fn imported_receiver_arguments_keep_exact_local_and_import_fact_ids() -> Result<(), Box<dyn Error>>
{
    let batch = batch()?;
    validate_evidence(&batch, EvidenceLimits::default())?;
    let good = batch
        .declarations
        .iter()
        .find(|fact| fact.qualified_name == "main.Good")
        .ok_or("missing Good declaration")?;
    let remote = batch
        .bindings
        .iter()
        .find(|fact| fact.spelling == "Remote")
        .ok_or("missing Remote import")?;
    let proofs = batch
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.relation == CandidateRelation::Calls && candidate.target_spelling == "lookup"
        })
        .filter_map(|candidate| candidate.constraints.typescript_type_arguments.as_ref())
        .collect::<Vec<_>>();
    assert_eq!(proofs.len(), 3);
    let local = Argument::Source {
        declaration_id: good.id.clone(),
        arguments: Vec::new(),
    };
    assert!(
        proofs
            .iter()
            .any(|proof| proof.arguments == vec![local.clone()])
    );
    assert!(
        proofs
            .iter()
            .any(|proof| proof.arguments == vec![Argument::Unknown])
    );
    assert!(proofs.iter().any(|proof| proof.arguments
        == vec![Argument::Import {
            binding_id: remote.id.clone(),
            member_path: Vec::new(),
            arguments: vec![local.clone()],
        }]));
    let encoded = serde_json::to_vec(&batch)?;
    assert_eq!(
        serde_json::from_slice::<SemanticEvidenceBatch>(&encoded)?,
        batch
    );
    Ok(())
}

#[test]
fn argument_proofs_reject_unknown_versions_and_invalid_fact_references()
-> Result<(), Box<dyn Error>> {
    let original = batch()?;
    for case in 0..6 {
        let mut batch = original.clone();
        let candidate = batch
            .candidates
            .iter_mut()
            .find(|candidate| {
                candidate.relation == CandidateRelation::Calls
                    && candidate
                        .constraints
                        .typescript_type_arguments
                        .as_ref()
                        .is_some_and(|proof| {
                            matches!(proof.arguments.as_slice(), [Argument::Source { .. }])
                        })
            })
            .ok_or("missing concrete argument proof")?;
        let proof = candidate
            .constraints
            .typescript_type_arguments
            .as_mut()
            .ok_or("missing proof")?;
        match case {
            0 => proof.version = 2,
            1 => proof.owner_binding_id = "missing-binding".to_owned(),
            2 => {
                proof.arguments = vec![Argument::Source {
                    declaration_id: "missing-declaration".to_owned(),
                    arguments: Vec::new(),
                }]
            }
            3 => {
                let Argument::Source { declaration_id, .. } = &proof.arguments[0] else {
                    return Err("expected source".into());
                };
                let declaration = batch
                    .declarations
                    .iter_mut()
                    .find(|fact| &fact.id == declaration_id)
                    .ok_or("missing source")?;
                declaration.namespace = Some(SymbolNamespace::Value);
            }
            4 => {
                proof.arguments = vec![Argument::Import {
                    binding_id: proof.owner_binding_id.clone(),
                    member_path: vec!["bad.path".to_owned()],
                    arguments: Vec::new(),
                }]
            }
            _ => {
                let Argument::Source { declaration_id, .. } = &proof.arguments[0] else {
                    return Err("expected source".into());
                };
                let declaration = batch
                    .declarations
                    .iter_mut()
                    .find(|fact| &fact.id == declaration_id)
                    .ok_or("missing source")?;
                declaration.kind = "type_parameter".to_owned();
            }
        }
        let error = validate_evidence(&batch, EvidenceLimits::default())
            .err()
            .ok_or("invalid proof accepted")?;
        assert!(
            matches!(
                error.code,
                EvidenceErrorCode::InvalidFact | EvidenceErrorCode::MissingReference
            ),
            "case {case}: {error}"
        );
    }
    Ok(())
}

#[test]
fn argument_proof_arity_is_bounded_and_unknown_is_distinct_from_absent()
-> Result<(), Box<dyn Error>> {
    let mut batch = batch()?;
    let candidate = batch
        .candidates
        .iter_mut()
        .find(|candidate| candidate.constraints.typescript_type_arguments.is_some())
        .ok_or("missing argument proof")?;
    let id = candidate.id.clone();
    candidate
        .constraints
        .typescript_type_arguments
        .as_mut()
        .ok_or("missing proof")?
        .arguments = vec![Argument::Unknown; 64];
    validate_evidence(&batch, EvidenceLimits::default())?;
    let candidate = batch
        .candidates
        .iter_mut()
        .find(|candidate| candidate.id == id)
        .ok_or("missing candidate")?;
    candidate
        .constraints
        .typescript_type_arguments
        .as_mut()
        .ok_or("missing proof")?
        .arguments
        .push(Argument::Unknown);
    assert_eq!(
        validate_evidence(&batch, EvidenceLimits::default())
            .err()
            .ok_or("oversized proof accepted")?
            .code,
        EvidenceErrorCode::ResourceLimit
    );
    let empty = serde_json::to_value(compass_languages::ResolutionConstraint::default())?;
    assert!(empty.get("typescriptTypeArguments").is_none());
    Ok(())
}

#[test]
fn absent_empty_and_unknown_argument_proofs_survive_json_distinctly() -> Result<(), Box<dyn Error>>
{
    let mut constraint = compass_languages::ResolutionConstraint::default();
    let absent = serde_json::to_value(&constraint)?;
    constraint.typescript_type_arguments = Some(compass_languages::TypeScriptTypeArguments {
        version: 1,
        owner_binding_id: "binding:root".to_owned(),
        arguments: Vec::new(),
    });
    let empty = serde_json::to_value(&constraint)?;
    constraint
        .typescript_type_arguments
        .as_mut()
        .ok_or("missing proof")?
        .arguments
        .push(Argument::Unknown);
    let unknown = serde_json::to_value(&constraint)?;
    assert_ne!(absent, empty);
    assert_ne!(empty, unknown);
    assert_ne!(absent, unknown);
    for encoded in [absent, empty, unknown] {
        let decoded: compass_languages::ResolutionConstraint =
            serde_json::from_value(encoded.clone())?;
        assert_eq!(serde_json::to_value(decoded)?, encoded);
    }
    Ok(())
}

#[test]
fn argument_proof_depth_and_candidate_nodes_have_inclusive_limits() -> Result<(), Box<dyn Error>> {
    let original = batch()?;
    let mut batch = original.clone();
    let candidate = batch
        .candidates
        .iter_mut()
        .find(|candidate| candidate.constraints.typescript_type_arguments.is_some())
        .ok_or("missing proof")?;
    let id = candidate.id.clone();
    let proof = candidate
        .constraints
        .typescript_type_arguments
        .as_mut()
        .ok_or("missing proof")?;
    let binding_id = proof.owner_binding_id.clone();
    let mut argument = Argument::Unknown;
    for _ in 0..16 {
        argument = Argument::Import {
            binding_id: binding_id.clone(),
            member_path: Vec::new(),
            arguments: vec![argument],
        };
    }
    proof.arguments = vec![argument.clone()];
    validate_evidence(&batch, EvidenceLimits::default())?;
    batch
        .candidates
        .iter_mut()
        .find(|candidate| candidate.id == id)
        .ok_or("missing candidate")?
        .constraints
        .typescript_type_arguments
        .as_mut()
        .ok_or("missing proof")?
        .arguments = vec![Argument::Import {
        binding_id: binding_id.clone(),
        member_path: Vec::new(),
        arguments: vec![argument],
    }];
    assert_eq!(
        validate_evidence(&batch, EvidenceLimits::default())
            .err()
            .ok_or("depth overflow accepted")?
            .code,
        EvidenceErrorCode::ResourceLimit
    );

    let mut batch = original;
    let proof = batch
        .candidates
        .iter_mut()
        .find_map(|candidate| candidate.constraints.typescript_type_arguments.as_mut())
        .ok_or("missing proof")?;
    proof.arguments = (0..4)
        .map(|_| Argument::Import {
            binding_id: binding_id.clone(),
            member_path: Vec::new(),
            arguments: vec![Argument::Unknown; 63],
        })
        .collect();
    validate_evidence(&batch, EvidenceLimits::default())?;
    let proof = batch
        .candidates
        .iter_mut()
        .find_map(|candidate| candidate.constraints.typescript_type_arguments.as_mut())
        .ok_or("missing proof")?;
    proof.arguments.push(Argument::Unknown);
    assert_eq!(
        validate_evidence(&batch, EvidenceLimits::default())
            .err()
            .ok_or("node overflow accepted")?
            .code,
        EvidenceErrorCode::ResourceLimit
    );
    Ok(())
}

#[test]
fn argument_proof_batch_nodes_are_bounded_before_reference_validation() -> Result<(), Box<dyn Error>>
{
    let mut batch = batch()?;
    let template = batch
        .candidates
        .iter()
        .find(|candidate| candidate.constraints.typescript_type_arguments.is_some())
        .ok_or("missing proof")?
        .clone();
    let binding_id = template.binding_id.clone().ok_or("missing root binding")?;
    batch.candidates.clear();
    for index in 0..782 {
        let mut candidate = template.clone();
        candidate.id = format!("candidate:batch-argument-limit:{index}");
        let proof = candidate
            .constraints
            .typescript_type_arguments
            .as_mut()
            .ok_or("missing proof")?;
        proof.arguments = if index == 781 {
            vec![Argument::Unknown; 64]
        } else {
            (0..4)
                .map(|_| Argument::Import {
                    binding_id: binding_id.clone(),
                    member_path: Vec::new(),
                    arguments: vec![Argument::Unknown; 63],
                })
                .collect()
        };
        batch.candidates.push(candidate);
    }
    // 781 * 256 + 64 = 200000 retained argument nodes.
    validate_evidence(&batch, EvidenceLimits::default())?;
    let mut extra = template;
    extra.id = "candidate:batch-argument-limit:extra".to_owned();
    extra
        .constraints
        .typescript_type_arguments
        .as_mut()
        .ok_or("missing proof")?
        .arguments = vec![Argument::Unknown];
    batch.candidates.push(extra);
    assert_eq!(
        validate_evidence(&batch, EvidenceLimits::default())
            .err()
            .ok_or("batch node overflow accepted")?
            .code,
        EvidenceErrorCode::ResourceLimit
    );
    Ok(())
}

#[test]
fn primitive_type_arguments_cannot_inherit_same_spelled_nominal_declarations()
-> Result<(), Box<dyn Error>> {
    let source = b"import type { Holder } from './provider'; class string { lookup() {} } class Owner { holder: Holder<string>; read() { this.holder.field.lookup(); } }";
    let batch = Engine::default().extract_source_universal_evidence(
        Path::new("main.ts"),
        "main.ts",
        source,
    )?;
    assert!(
        batch
            .declarations
            .iter()
            .any(|declaration| declaration.kind == "class" && declaration.name == "string")
    );
    let proof = batch
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.relation == CandidateRelation::Calls && candidate.target_spelling == "lookup"
        })
        .find_map(|candidate| candidate.constraints.typescript_type_arguments.as_ref())
        .ok_or("missing primitive argument proof")?;
    assert_eq!(proof.arguments, vec![Argument::Unknown]);
    validate_evidence(&batch, EvidenceLimits::default())?;
    Ok(())
}

#[test]
fn observed_merge_loads_are_typed_anchored_and_round_trip() -> Result<(), Box<dyn Error>> {
    use compass_languages::TypeScriptMergeDependencyKind as Kind;
    let source = br#"/// <reference path="./augment.d.ts" />
/// <reference types="augment" />
import './side'; import type { Item } from './types'; export * from './barrel';
import Old = require('./old');
export function read() { require('./require'); import('./dynamic'); }
"#;
    let batch = Engine::default().extract_source_universal_evidence(
        Path::new("main.ts"),
        "main.ts",
        source,
    )?;
    assert_eq!(batch.typescript_merge_inventory_complete, Some(true));
    let mut expected = vec![
        (Kind::ReferencePath, "./augment.d.ts"),
        (Kind::ReferenceTypes, "augment"),
        (Kind::Module, "./side"),
        (Kind::Module, "./types"),
        (Kind::Module, "./barrel"),
        (Kind::Module, "./old"),
        (Kind::Module, "./require"),
        (Kind::Module, "./dynamic"),
    ];
    let mut actual = batch
        .typescript_merge_dependencies
        .iter()
        .map(|dependency| {
            assert_eq!(dependency.range.source_file, "main.ts");
            assert!(dependency.range.start_byte < dependency.range.end_byte);
            (dependency.kind, dependency.specifier.as_str())
        })
        .collect::<Vec<_>>();
    expected.sort();
    actual.sort();
    assert_eq!(actual, expected);
    validate_evidence(&batch, EvidenceLimits::default())?;
    assert_eq!(
        serde_json::from_slice::<SemanticEvidenceBatch>(&serde_json::to_vec(&batch)?)?,
        batch
    );
    let mut wrong = batch.clone();
    wrong.typescript_merge_dependencies[0].range.source_file = "other.ts".to_owned();
    assert!(validate_evidence(&wrong, EvidenceLimits::default()).is_err());
    for source in [
        r#"/// <reference lib="esnext" />"#,
        "export function read(path: string) { import(path); }",
        r#"export {}; import './\u0061ugment';"#,
    ] {
        let batch = Engine::default().extract_source_universal_evidence(
            Path::new("main.ts"),
            "main.ts",
            source.as_bytes(),
        )?;
        assert_eq!(
            batch.typescript_merge_inventory_complete,
            Some(false),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn merge_literal_census_preserves_interior_quote_identity() -> Result<(), Box<dyn Error>> {
    let source = br#"import './end"';
export * from "./end'";
export const loaded = import('./end`');
import "'./start";
export * from '`./start';
export const required = require('"./start');
"#;
    let batch = Engine::default().extract_source_universal_evidence(
        Path::new("main.ts"),
        "main.ts",
        source,
    )?;
    assert_eq!(batch.typescript_merge_inventory_complete, Some(true));
    for dependency in &batch.typescript_merge_dependencies {
        let start = usize::try_from(dependency.range.start_byte)?;
        let end = usize::try_from(dependency.range.end_byte)?;
        assert_eq!(
            std::str::from_utf8(&source[start + 1..end - 1])?,
            dependency.specifier
        );
    }

    let mut actual = batch
        .typescript_merge_dependencies
        .iter()
        .map(|dependency| dependency.specifier.as_str())
        .collect::<Vec<_>>();
    let mut expected = vec![
        r#"./end""#,
        "./end'",
        "./end`",
        "'./start",
        "`./start",
        r#""./start"#,
    ];
    actual.sort();
    expected.sort();
    assert_eq!(actual, expected);
    Ok(())
}

#[test]
fn merge_census_version_is_explicit_and_unknown_major_is_an_error() -> Result<(), Box<dyn Error>> {
    for source in ["", "// empty source", "export class Item {}"] {
        let original = Engine::default().extract_source_universal_evidence(
            Path::new("main.ts"),
            "main.ts",
            source.as_bytes(),
        )?;
        assert_eq!(
            original.typescript_merge_inventory_version,
            Some(compass_languages::TYPESCRIPT_MERGE_INVENTORY_VERSION)
        );
        let mut wire = serde_json::to_value(&original)?;
        wire.as_object_mut()
            .ok_or("missing batch object")?
            .remove("typescriptMergeInventoryVersion");
        let legacy: SemanticEvidenceBatch = serde_json::from_value(wire)?;
        assert_eq!(legacy.typescript_merge_inventory_version, None);
        validate_evidence(&legacy, EvidenceLimits::default())?;
        let mut future = original;
        future.typescript_merge_inventory_version = Some(2);
        assert!(
            matches!(validate_evidence(&future, EvidenceLimits::default()), Err(error) if error.code == EvidenceErrorCode::InvalidFact)
        );
    }
    Ok(())
}

#[test]
fn export_census_preserves_unresolved_names_and_excludes_partial_surfaces()
-> Result<(), Box<dyn Error>> {
    use compass_languages::{BindingKind, ECMASCRIPT_EXPORT_INVENTORY_VERSION};
    for (source, expected) in [
        ("", true),
        ("// empty source module", true),
        ("export {};", true),
        ("export interface Holder<T> { field: T }", true),
        (
            "interface Local<T> { field: T } export { Local as Holder };",
            true,
        ),
        (
            "export { Missing as Holder }; export * from './provider';",
            true,
        ),
        ("export const value = 1;", true),
        ("export { default as Holder } from './provider';", true),
        ("export default class Named {}", true),
        ("export const { key: value } = source;", false),
        ("export * from './provider'; export = missing;", false),
        ("export * from './provider'; export { Broken as", false),
        ("export { Missing as 'Holder' };", false),
        (
            "declare module 'external' { export interface Holder {} }",
            false,
        ),
        ("module.exports = {};", false),
        (
            "export namespace Nested { export interface Holder {} }",
            false,
        ),
    ] {
        for file in ["main.ts", "main.js"] {
            // TypeScript-only syntax is deliberately tested with its grammar.
            if file.ends_with(".js")
                && (source.contains("interface")
                    || source.contains("declare ")
                    || source.contains("export =")
                    || source.contains("namespace"))
            {
                continue;
            }
            let batch = Engine::default().extract_source_universal_evidence(
                Path::new(file),
                file,
                source.as_bytes(),
            )?;
            validate_evidence(&batch, EvidenceLimits::default())?;
            let root = batch
                .declarations
                .iter()
                .find(|fact| {
                    fact.scope_id.is_none() && matches!(fact.kind.as_str(), "file" | "module")
                })
                .ok_or("missing source root")?;
            let inventory = root
                .ecmascript_export_inventory
                .ok_or("missing source census")?;
            assert_eq!(inventory.version, ECMASCRIPT_EXPORT_INVENTORY_VERSION);
            assert_eq!(inventory.complete, expected, "{file}: {source}");
            if source.contains("Missing as Holder") {
                let binding = batch
                    .bindings
                    .iter()
                    .find(|binding| {
                        binding.kind == BindingKind::Reexport && binding.spelling == "Holder"
                    })
                    .ok_or("lost unresolved explicit export")?;
                assert_eq!(binding.qualified_target, "Missing");
                assert!(binding.target_declaration_id.is_none());
                assert_eq!(
                    &source[binding.range.start_byte as usize..binding.range.end_byte as usize],
                    "Holder"
                );
            }
            assert_eq!(
                serde_json::from_slice::<SemanticEvidenceBatch>(&serde_json::to_vec(&batch)?)?,
                batch
            );
        }
    }
    Ok(())
}

#[test]
fn export_inventory_validation_rejects_unknown_versions_and_wrong_owners()
-> Result<(), Box<dyn Error>> {
    let original = batch()?;
    let root = original
        .declarations
        .iter()
        .position(|fact| fact.scope_id.is_none())
        .ok_or("missing root")?;
    let class = original
        .declarations
        .iter()
        .position(|fact| fact.kind == "class")
        .ok_or("missing class")?;
    let inventory = original.declarations[root]
        .ecmascript_export_inventory
        .ok_or("missing export inventory")?;
    for case in 0..4 {
        let mut changed = original.clone();
        match case {
            0 => {
                changed.declarations[root]
                    .ecmascript_export_inventory
                    .as_mut()
                    .ok_or("missing census")?
                    .version += 1
            }
            1 => changed.declarations[class].ecmascript_export_inventory = Some(inventory),
            2 => changed.declarations[root].kind = "namespace".to_owned(),
            _ => {
                changed.pipeline.language = "python".to_owned();
                changed.declarations[root].language = "python".to_owned();
            }
        }
        assert!(
            validate_evidence(&changed, EvidenceLimits::default()).is_err(),
            "case {case}"
        );
    }
    let mut legacy = original.clone();
    legacy.declarations[root].ecmascript_export_inventory = None;
    validate_evidence(&legacy, EvidenceLimits::default())?;
    let wire = serde_json::to_string(&legacy)?;
    assert!(!wire.contains("ecmascriptExportInventory"));
    assert_eq!(
        serde_json::from_str::<SemanticEvidenceBatch>(&wire)?,
        legacy
    );
    Ok(())
}

#[test]
fn keyword_default_reexport_preserves_imported_identity_in_both_grammars()
-> Result<(), Box<dyn Error>> {
    let source = "export { default as Holder } from './provider';";
    for file in ["main.ts", "main.js"] {
        let batch = Engine::default().extract_source_universal_evidence(
            Path::new(file),
            file,
            source.as_bytes(),
        )?;
        let binding = batch
            .bindings
            .iter()
            .find(|binding| {
                binding.kind == compass_languages::BindingKind::Reexport
                    && binding.spelling == "Holder"
            })
            .ok_or("missing Holder export")?;
        assert_eq!(binding.qualified_target, "./provider::default", "{file}");
        assert_eq!(
            &source[binding.range.start_byte as usize..binding.range.end_byte as usize],
            "Holder"
        );
    }
    Ok(())
}
