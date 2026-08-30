use std::collections::{BTreeMap, BTreeSet};

use compass_languages::{
    CandidateRelation, EvidenceRange, Extraction, RawEdgeRecord, RelationshipCandidate,
};
use compass_model::provenance::OCCURRENCE_RULE_ATTRIBUTE;
use compass_program::CompilerProjection;
use serde_json::{Map, Value};

type AnchorKey = (String, u64, u64);

#[derive(Clone, Debug, Default)]
pub struct ProgramProjectionSites {
    declarations: BTreeMap<AnchorKey, Vec<DeclarationSite>>,
    calls: BTreeMap<AnchorKey, Vec<CallSite>>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct DeclarationSite {
    evidence_id: String,
    kind: String,
    language: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CallSite {
    candidate_id: String,
    source_declaration_id: String,
    allowed_target_kinds: Vec<String>,
    language: String,
    range: EvidenceRange,
}

/// Retain the small subset of AST evidence needed for a later compiler join.
#[must_use]
pub fn collect_program_projection_sites(extractions: &[Extraction]) -> ProgramProjectionSites {
    let mut sites = ProgramProjectionSites::default();
    for batch in extractions
        .iter()
        .filter_map(|extraction| extraction.semantic_evidence.as_ref())
        .filter(|batch| matches!(batch.pipeline.language.as_str(), "java" | "python"))
    {
        for declaration in &batch.declarations {
            sites
                .declarations
                .entry(anchor_key(&declaration.range))
                .or_default()
                .push(DeclarationSite {
                    evidence_id: declaration.id.clone(),
                    kind: declaration.kind.clone(),
                    language: batch.pipeline.language.clone(),
                });
        }
        let occurrences = batch
            .occurrences
            .iter()
            .map(|occurrence| (occurrence.id.as_str(), &occurrence.range))
            .collect::<BTreeMap<_, _>>();
        for candidate in
            batch
                .candidates
                .iter()
                .filter(|candidate| match batch.pipeline.language.as_str() {
                    "java" => matches!(
                        candidate.relation,
                        CandidateRelation::Calls | CandidateRelation::Constructs
                    ),
                    "python" => candidate.relation == CandidateRelation::Calls,
                    _ => false,
                })
        {
            let Some(range) = candidate
                .occurrence_id
                .as_deref()
                .and_then(|id| occurrences.get(id).copied())
            else {
                continue;
            };
            sites
                .calls
                .entry(anchor_key(range))
                .or_default()
                .push(call_site(candidate, range, &batch.pipeline.language));
        }
    }
    for values in sites.declarations.values_mut() {
        values.sort();
        values.dedup();
    }
    for values in sites.calls.values_mut() {
        values.sort_by(|left, right| {
            (
                &left.candidate_id,
                &left.source_declaration_id,
                &left.allowed_target_kinds,
                &left.language,
                &left.range.source_file,
                left.range.start_byte,
                left.range.end_byte,
            )
                .cmp(&(
                    &right.candidate_id,
                    &right.source_declaration_id,
                    &right.allowed_target_kinds,
                    &right.language,
                    &right.range.source_file,
                    right.range.start_byte,
                    right.range.end_byte,
                ))
        });
        values.dedup();
    }
    sites
}

/// Apply fresh compiler identities only where an exact AST anchor proves a call.
///
/// Conflicting providers, ambiguous anchors, non-local targets, and non-call
/// references are deliberately left to the structural resolver.
pub fn apply_program_projection(
    extraction: &mut Extraction,
    sites: &ProgramProjectionSites,
    projection: &CompilerProjection,
) {
    let graph_ids = extraction
        .nodes
        .iter()
        .filter_map(|node| {
            node.attributes
                .get("evidence_declaration_id")
                .and_then(Value::as_str)
                .map(|id| (id.to_owned(), node.id.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    let declaration_sites = sites
        .declarations
        .values()
        .flatten()
        .map(|site| (site.evidence_id.as_str(), site))
        .collect::<BTreeMap<_, _>>();

    let mut symbols = BTreeMap::<(String, String), BTreeMap<String, BTreeSet<String>>>::new();
    for definition in &projection.definitions {
        let key = (
            definition.anchor.source_file.clone(),
            definition.anchor.start_byte,
            definition.anchor.end_byte,
        );
        let Some([site]) = sites.declarations.get(&key).map(Vec::as_slice) else {
            continue;
        };
        if !provider_is_admitted(&site.language, &definition.provider_id) {
            continue;
        }
        symbols
            .entry((site.language.clone(), definition.symbol.clone()))
            .or_default()
            .entry(site.evidence_id.clone())
            .or_default()
            .insert(definition.provider_id.clone());
    }

    let mut calls = BTreeMap::<AnchorKey, BTreeMap<String, BTreeSet<String>>>::new();
    for call in &projection.calls {
        let key = (
            call.anchor.source_file.clone(),
            call.anchor.start_byte,
            call.anchor.end_byte,
        );
        let Some([site]) = sites.calls.get(&key).map(Vec::as_slice) else {
            continue;
        };
        if !provider_is_admitted(&site.language, &call.provider_id) {
            continue;
        }
        calls
            .entry(key)
            .or_default()
            .entry(call.target.clone())
            .or_default()
            .insert(call.provider_id.clone());
    }

    for (anchor, targets) in calls {
        let target_entries = targets.into_iter().collect::<Vec<_>>();
        let [(target_symbol, call_providers)] = target_entries.as_slice() else {
            continue;
        };
        let Some([call_site]) = sites.calls.get(&anchor).map(Vec::as_slice) else {
            continue;
        };
        let Some(symbol_targets) =
            symbols.get(&(call_site.language.clone(), target_symbol.clone()))
        else {
            continue;
        };
        let target_declarations = symbol_targets.iter().collect::<Vec<_>>();
        let [(target_declaration, definition_providers)] = target_declarations.as_slice() else {
            continue;
        };
        if call_site.language == "python" && call_providers.is_disjoint(definition_providers) {
            continue;
        }
        let Some(target_site) = declaration_sites.get(target_declaration.as_str()).copied() else {
            continue;
        };
        if !call_site.allowed_target_kinds.is_empty()
            && !call_site.allowed_target_kinds.contains(&target_site.kind)
        {
            continue;
        }
        let (Some(source), Some(target)) = (
            graph_ids.get(&call_site.source_declaration_id),
            graph_ids.get(*target_declaration),
        ) else {
            continue;
        };
        extraction.edges.retain(|edge| {
            !(edge.source == *source
                && edge.attributes.get("relation").and_then(Value::as_str) == Some("calls")
                && edge.attributes.get("source_file").and_then(Value::as_str)
                    == Some(call_site.range.source_file.as_str())
                && edge.attributes.get("start_byte").and_then(Value::as_u64)
                    == Some(call_site.range.start_byte)
                && edge.attributes.get("end_byte").and_then(Value::as_u64)
                    == Some(call_site.range.end_byte))
        });
        extraction.edges.push(compiler_edge(
            source.clone(),
            target.clone(),
            call_site,
            call_providers,
        ));
    }
    extraction.edges.sort_by_cached_key(|edge| {
        (
            edge.source.clone(),
            edge.target.clone(),
            Value::Object(edge.attributes.clone()).to_string(),
        )
    });
    extraction.edges.dedup();
}

fn call_site(candidate: &RelationshipCandidate, range: &EvidenceRange, language: &str) -> CallSite {
    let mut allowed_target_kinds = candidate.constraints.allowed_target_kinds.clone();
    allowed_target_kinds.sort();
    allowed_target_kinds.dedup();
    CallSite {
        candidate_id: candidate.id.clone(),
        source_declaration_id: candidate.source_declaration_id.clone(),
        allowed_target_kinds,
        language: language.to_owned(),
        range: range.clone(),
    }
}

fn provider_is_admitted(language: &str, provider_id: &str) -> bool {
    language != "python" || managed_python_profile_digest(provider_id).is_some()
}

fn managed_python_profile_digest(provider_id: &str) -> Option<&str> {
    let mut parts = provider_id.split(':');
    let (Some("scip-python"), Some(profile), Some(artifact), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    if profile.len() == 64
        && artifact.len() == 64
        && profile
            .bytes()
            .chain(artifact.bytes())
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Some(profile)
    } else {
        None
    }
}

fn anchor_key(range: &EvidenceRange) -> AnchorKey {
    (range.source_file.clone(), range.start_byte, range.end_byte)
}

fn compiler_edge(
    source: String,
    target: String,
    site: &CallSite,
    providers: &BTreeSet<String>,
) -> RawEdgeRecord {
    let (rule, extractor) = if site.language == "python" {
        (
            "scip-python-exact-anchor",
            "compass.resolve.python.scip-python",
        )
    } else {
        ("compiler-exact-anchor", "compass.resolve.java.program")
    };
    let mut attributes = Map::from_iter([
        ("relation".to_owned(), Value::String("calls".to_owned())),
        ("_origin".to_owned(), Value::String("artifact".to_owned())),
        (
            "confidence".to_owned(),
            Value::String("EXTRACTED".to_owned()),
        ),
        (
            "source_file".to_owned(),
            Value::String(site.range.source_file.clone()),
        ),
        (
            "source_location".to_owned(),
            Value::String(format!("L{}", site.range.start_line)),
        ),
        ("start_byte".to_owned(), Value::from(site.range.start_byte)),
        ("end_byte".to_owned(), Value::from(site.range.end_byte)),
        ("line_start".to_owned(), Value::from(site.range.start_line)),
        ("line_end".to_owned(), Value::from(site.range.end_line)),
        (
            "column_start".to_owned(),
            Value::from(site.range.start_column),
        ),
        ("column_end".to_owned(), Value::from(site.range.end_column)),
        ("weight".to_owned(), Value::from(1.0)),
        ("language".to_owned(), Value::String(site.language.clone())),
        ("extractor".to_owned(), Value::String(extractor.to_owned())),
        ("resolution_rule".to_owned(), Value::String(rule.to_owned())),
        ("rule".to_owned(), Value::String(rule.to_owned())),
        (
            OCCURRENCE_RULE_ATTRIBUTE.to_owned(),
            Value::String(rule.to_owned()),
        ),
        (
            "evidence_candidate_id".to_owned(),
            Value::String(site.candidate_id.clone()),
        ),
    ]);
    if site.language == "python" {
        attributes.insert(
            "analyzer_providers".to_owned(),
            Value::Array(providers.iter().cloned().map(Value::String).collect()),
        );
        attributes.insert(
            "analyzer_profiles".to_owned(),
            Value::Array(
                providers
                    .iter()
                    .filter_map(|provider| managed_python_profile_digest(provider))
                    .map(str::to_owned)
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .map(Value::String)
                    .collect(),
            ),
        );
    }
    RawEdgeRecord {
        source,
        target,
        attributes,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::error::Error;
    use std::path::Path;

    use compass_languages::Engine;
    use compass_program::{CompilerCall, CompilerDefinition};

    use super::*;
    use crate::resolve_owned_with_root;

    const SOURCE_FILE: &str = "src/Demo.java";
    const SOURCE: &str = "class Demo {\n  void pick(int x) {}\n  void pick(String x) {}\n  void use() { pick(\"x\"); }\n}\n";
    const PYTHON_PROVIDER: &str = "scip-python:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    #[test]
    fn exact_compiler_symbol_resolves_java_overload() -> Result<(), Box<dyn Error>> {
        let extraction =
            Engine::default().extract_source(Path::new(SOURCE_FILE), SOURCE.as_bytes())?;
        let evidence = extraction
            .semantic_evidence
            .as_ref()
            .ok_or("missing Java evidence")?;
        let target = evidence
            .declarations
            .iter()
            .find(|declaration| {
                declaration.name == "pick"
                    && declaration
                        .signature
                        .as_deref()
                        .is_some_and(|signature| signature.contains("String"))
            })
            .ok_or("missing String overload")?;
        let call = evidence
            .candidates
            .iter()
            .find(|candidate| candidate.relation == CandidateRelation::Calls)
            .ok_or("missing call candidate")?;
        let occurrence = evidence
            .occurrences
            .iter()
            .find(|occurrence| Some(occurrence.id.as_str()) == call.occurrence_id.as_deref())
            .ok_or("missing call occurrence")?;
        let target_node = target.graph_node_id.clone();
        let projection = CompilerProjection {
            definitions: vec![CompilerDefinition {
                provider_id: "scip/java".to_owned(),
                symbol: "java maven fixture Demo#pick(java.lang.String).".to_owned(),
                anchor: compiler_anchor(&target.range),
            }],
            calls: vec![CompilerCall {
                provider_id: "scip/java".to_owned(),
                target: "java maven fixture Demo#pick(java.lang.String).".to_owned(),
                anchor: compiler_anchor(&occurrence.range),
            }],
        };
        let sites = collect_program_projection_sites(std::slice::from_ref(&extraction));
        let mut resolved =
            resolve_owned_with_root(vec![extraction], &HashMap::new(), Path::new("/repo"));

        apply_program_projection(&mut resolved, &sites, &projection);

        assert!(resolved.edges.iter().any(|edge| {
            edge.target == target_node
                && edge.attributes.get("extractor").and_then(Value::as_str)
                    == Some("compass.resolve.java.program")
        }));
        Ok(())
    }

    #[test]
    fn compiler_reference_must_be_an_ast_call_and_providers_must_agree()
    -> Result<(), Box<dyn Error>> {
        let extraction =
            Engine::default().extract_source(Path::new(SOURCE_FILE), SOURCE.as_bytes())?;
        let evidence = extraction
            .semantic_evidence
            .as_ref()
            .ok_or("missing Java evidence")?;
        let definitions = evidence
            .declarations
            .iter()
            .filter(|declaration| declaration.name == "pick")
            .collect::<Vec<_>>();
        let [first, second] = definitions.as_slice() else {
            return Err("expected two overloads".into());
        };
        let call = evidence
            .candidates
            .iter()
            .find(|candidate| candidate.relation == CandidateRelation::Calls)
            .ok_or("missing call candidate")?;
        let call_range = evidence
            .occurrences
            .iter()
            .find(|occurrence| Some(occurrence.id.as_str()) == call.occurrence_id.as_deref())
            .map(|occurrence| &occurrence.range)
            .ok_or("missing call occurrence")?;
        let type_reference = evidence
            .occurrences
            .iter()
            .find(|occurrence| occurrence.spelling == "String")
            .ok_or("missing type reference")?;
        let projection = CompilerProjection {
            definitions: vec![
                CompilerDefinition {
                    provider_id: "scip/one".to_owned(),
                    symbol: "first".to_owned(),
                    anchor: compiler_anchor(&first.range),
                },
                CompilerDefinition {
                    provider_id: "scip/two".to_owned(),
                    symbol: "second".to_owned(),
                    anchor: compiler_anchor(&second.range),
                },
            ],
            calls: vec![
                CompilerCall {
                    provider_id: "scip/one".to_owned(),
                    target: "first".to_owned(),
                    anchor: compiler_anchor(call_range),
                },
                CompilerCall {
                    provider_id: "scip/two".to_owned(),
                    target: "second".to_owned(),
                    anchor: compiler_anchor(call_range),
                },
                CompilerCall {
                    provider_id: "scip/one".to_owned(),
                    target: "second".to_owned(),
                    anchor: compiler_anchor(&type_reference.range),
                },
            ],
        };
        let sites = collect_program_projection_sites(std::slice::from_ref(&extraction));
        let mut resolved =
            resolve_owned_with_root(vec![extraction], &HashMap::new(), Path::new("/repo"));

        apply_program_projection(&mut resolved, &sites, &projection);

        assert!(resolved.edges.iter().all(|edge| {
            edge.attributes.get("extractor").and_then(Value::as_str)
                != Some("compass.resolve.java.program")
        }));
        Ok(())
    }

    #[test]
    fn exact_compiler_symbol_preserves_recursive_java_call() -> Result<(), Box<dyn Error>> {
        let source = "class Recursive { void run() { run(); } }\n";
        let extraction =
            Engine::default().extract_source(Path::new(SOURCE_FILE), source.as_bytes())?;
        let evidence = extraction
            .semantic_evidence
            .as_ref()
            .ok_or("missing Java evidence")?;
        let declaration = evidence
            .declarations
            .iter()
            .find(|declaration| declaration.name == "run")
            .ok_or("missing run declaration")?;
        let candidate = evidence
            .candidates
            .iter()
            .find(|candidate| candidate.relation == CandidateRelation::Calls)
            .ok_or("missing recursive call")?;
        let occurrence = evidence
            .occurrences
            .iter()
            .find(|occurrence| Some(occurrence.id.as_str()) == candidate.occurrence_id.as_deref())
            .ok_or("missing recursive occurrence")?;
        let projection = CompilerProjection {
            definitions: vec![CompilerDefinition {
                provider_id: "scip/java".to_owned(),
                symbol: "recursive".to_owned(),
                anchor: compiler_anchor(&declaration.range),
            }],
            calls: vec![CompilerCall {
                provider_id: "scip/java".to_owned(),
                target: "recursive".to_owned(),
                anchor: compiler_anchor(&occurrence.range),
            }],
        };
        let graph_node_id = declaration.graph_node_id.clone();
        let sites = collect_program_projection_sites(std::slice::from_ref(&extraction));
        let mut resolved =
            resolve_owned_with_root(vec![extraction], &HashMap::new(), Path::new("/repo"));

        apply_program_projection(&mut resolved, &sites, &projection);

        assert!(resolved.edges.iter().any(|edge| {
            edge.source == graph_node_id
                && edge.target == graph_node_id
                && edge.attributes.get("extractor").and_then(Value::as_str)
                    == Some("compass.resolve.java.program")
        }));
        Ok(())
    }

    #[test]
    fn managed_python_exact_anchors_qualify_typed_call_families() -> Result<(), Box<dyn Error>> {
        const PYTHON_FILE: &str = "src/typed_calls.py";
        const PYTHON_SOURCE: &str = r#"from typing import Callable, Protocol, overload

def typed_target() -> None: pass
def protocol_target() -> None: pass
def property_target() -> None: pass
def callable_target() -> None: pass
@overload
def overload_target(value: int) -> int: ...
@overload
def overload_target(value: str) -> str: ...
def overload_target(value): return value
def callback_target() -> None: pass
def return_target() -> None: pass

def exercise() -> None:
    typed_target()
    protocol_target()
    property_target()
    callable_target()
    overload_target(1)
    callback_target()
    return_target()
"#;
        let extraction =
            Engine::default().extract_source(Path::new(PYTHON_FILE), PYTHON_SOURCE.as_bytes())?;
        let evidence = extraction
            .semantic_evidence
            .as_ref()
            .ok_or("missing Python evidence")?;
        let families = [
            "typed_target",
            "protocol_target",
            "property_target",
            "callable_target",
            "overload_target",
            "callback_target",
            "return_target",
        ];
        let mut definitions = Vec::new();
        let mut calls = Vec::new();
        for family in families {
            let declaration = evidence
                .declarations
                .iter()
                .find(|declaration| declaration.name == family)
                .ok_or("missing typed-family declaration")?;
            let candidate = evidence
                .candidates
                .iter()
                .find(|candidate| {
                    candidate.relation == CandidateRelation::Calls
                        && candidate.target_spelling == family
                })
                .ok_or("missing typed-family call")?;
            let occurrence = evidence
                .occurrences
                .iter()
                .find(|occurrence| {
                    Some(occurrence.id.as_str()) == candidate.occurrence_id.as_deref()
                })
                .ok_or("missing typed-family occurrence")?;
            let symbol = format!("python fixture {family}");
            definitions.push(CompilerDefinition {
                provider_id: PYTHON_PROVIDER.to_owned(),
                symbol: symbol.clone(),
                anchor: compiler_anchor(&declaration.range),
            });
            calls.push(CompilerCall {
                provider_id: PYTHON_PROVIDER.to_owned(),
                target: symbol,
                anchor: compiler_anchor(&occurrence.range),
            });
        }
        let projection = CompilerProjection { definitions, calls };
        let sites = collect_program_projection_sites(std::slice::from_ref(&extraction));
        let mut resolved =
            resolve_owned_with_root(vec![extraction], &HashMap::new(), Path::new("/repo"));

        apply_program_projection(&mut resolved, &sites, &projection);

        let projected = resolved
            .edges
            .iter()
            .filter(|edge| {
                edge.attributes.get("extractor").and_then(Value::as_str)
                    == Some("compass.resolve.python.scip-python")
            })
            .collect::<Vec<_>>();
        assert_eq!(projected.len(), families.len());
        assert!(projected.iter().all(|edge| {
            edge.attributes.get("rule").and_then(Value::as_str) == Some("scip-python-exact-anchor")
                && edge.attributes.get("analyzer_profiles")
                    == Some(&serde_json::json!([
                        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    ]))
        }));
        Ok(())
    }

    #[test]
    fn python_projection_rejects_generic_conflicting_and_inexact_artifacts()
    -> Result<(), Box<dyn Error>> {
        const PYTHON_FILE: &str = "src/app.py";
        const PYTHON_SOURCE: &str = "def target(): pass\ndef other(): pass\ndef run(): target()\n";
        let extraction =
            Engine::default().extract_source(Path::new(PYTHON_FILE), PYTHON_SOURCE.as_bytes())?;
        let evidence = extraction
            .semantic_evidence
            .as_ref()
            .ok_or("missing Python evidence")?;
        let target = evidence
            .declarations
            .iter()
            .find(|declaration| declaration.name == "target")
            .ok_or("missing target")?;
        let other = evidence
            .declarations
            .iter()
            .find(|declaration| declaration.name == "other")
            .ok_or("missing other")?;
        let candidate = evidence
            .candidates
            .iter()
            .find(|candidate| candidate.relation == CandidateRelation::Calls)
            .ok_or("missing call")?;
        let occurrence = evidence
            .occurrences
            .iter()
            .find(|occurrence| Some(occurrence.id.as_str()) == candidate.occurrence_id.as_deref())
            .ok_or("missing call occurrence")?;
        let target_anchor = compiler_anchor(&target.range);
        let other_anchor = compiler_anchor(&other.range);
        let occurrence_anchor = compiler_anchor(&occurrence.range);
        let sites = collect_program_projection_sites(std::slice::from_ref(&extraction));
        let base = resolve_owned_with_root(vec![extraction], &HashMap::new(), Path::new("/repo"));

        let mut generic = base.clone();
        apply_program_projection(
            &mut generic,
            &sites,
            &CompilerProjection {
                definitions: vec![CompilerDefinition {
                    provider_id: "scip:generic".to_owned(),
                    symbol: "target".to_owned(),
                    anchor: target_anchor.clone(),
                }],
                calls: vec![CompilerCall {
                    provider_id: "scip:generic".to_owned(),
                    target: "target".to_owned(),
                    anchor: occurrence_anchor.clone(),
                }],
            },
        );
        assert!(generic.edges.iter().all(|edge| {
            edge.attributes.get("extractor").and_then(Value::as_str)
                != Some("compass.resolve.python.scip-python")
        }));

        let mut conflicted = base.clone();
        apply_program_projection(
            &mut conflicted,
            &sites,
            &CompilerProjection {
                definitions: vec![
                    CompilerDefinition {
                        provider_id: PYTHON_PROVIDER.to_owned(),
                        symbol: "first".to_owned(),
                        anchor: target_anchor,
                    },
                    CompilerDefinition {
                        provider_id: PYTHON_PROVIDER.to_owned(),
                        symbol: "second".to_owned(),
                        anchor: other_anchor,
                    },
                ],
                calls: vec![
                    CompilerCall {
                        provider_id: PYTHON_PROVIDER.to_owned(),
                        target: "first".to_owned(),
                        anchor: occurrence_anchor.clone(),
                    },
                    CompilerCall {
                        provider_id: PYTHON_PROVIDER.to_owned(),
                        target: "second".to_owned(),
                        anchor: occurrence_anchor.clone(),
                    },
                    CompilerCall {
                        provider_id: PYTHON_PROVIDER.to_owned(),
                        target: "first".to_owned(),
                        anchor: compass_ir::SourceAnchor {
                            start_byte: occurrence_anchor.start_byte + 1,
                            ..occurrence_anchor
                        },
                    },
                ],
            },
        );
        assert!(conflicted.edges.iter().all(|edge| {
            edge.attributes.get("extractor").and_then(Value::as_str)
                != Some("compass.resolve.python.scip-python")
        }));
        Ok(())
    }

    fn compiler_anchor(range: &EvidenceRange) -> compass_ir::SourceAnchor {
        compass_ir::SourceAnchor {
            source_file: range.source_file.clone(),
            start_byte: range.start_byte,
            end_byte: range.end_byte,
        }
    }
}
