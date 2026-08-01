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
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CallSite {
    candidate_id: String,
    source_declaration_id: String,
    allowed_target_kinds: Vec<String>,
    range: EvidenceRange,
}

/// Retain the small subset of Java AST evidence needed for a later compiler join.
#[must_use]
pub fn collect_program_projection_sites(extractions: &[Extraction]) -> ProgramProjectionSites {
    let mut sites = ProgramProjectionSites::default();
    for batch in extractions
        .iter()
        .filter_map(|extraction| extraction.semantic_evidence.as_ref())
        .filter(|batch| batch.adapter.language == "java")
    {
        for declaration in &batch.declarations {
            sites
                .declarations
                .entry(anchor_key(&declaration.range))
                .or_default()
                .push(DeclarationSite {
                    evidence_id: declaration.id.clone(),
                    kind: declaration.kind.clone(),
                });
        }
        let occurrences = batch
            .occurrences
            .iter()
            .map(|occurrence| (occurrence.id.as_str(), &occurrence.range))
            .collect::<BTreeMap<_, _>>();
        for candidate in batch.candidates.iter().filter(|candidate| {
            matches!(
                candidate.relation,
                CandidateRelation::Calls | CandidateRelation::Constructs
            )
        }) {
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
                .push(call_site(candidate, range));
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
                &left.range.source_file,
                left.range.start_byte,
                left.range.end_byte,
            )
                .cmp(&(
                    &right.candidate_id,
                    &right.source_declaration_id,
                    &right.allowed_target_kinds,
                    &right.range.source_file,
                    right.range.start_byte,
                    right.range.end_byte,
                ))
        });
        values.dedup();
    }
    sites
}

/// Apply fresh compiler identities only where exact Java AST evidence proves a call.
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

    let mut symbols = BTreeMap::<String, BTreeMap<String, BTreeSet<String>>>::new();
    for definition in &projection.definitions {
        let key = (
            definition.anchor.source_file.clone(),
            definition.anchor.start_byte,
            definition.anchor.end_byte,
        );
        let Some([site]) = sites.declarations.get(&key).map(Vec::as_slice) else {
            continue;
        };
        symbols
            .entry(definition.symbol.clone())
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
        calls
            .entry(key)
            .or_default()
            .entry(call.target.clone())
            .or_default()
            .insert(call.provider_id.clone());
    }

    for (anchor, targets) in calls {
        let target_entries = targets.into_iter().collect::<Vec<_>>();
        let [(target_symbol, _call_providers)] = target_entries.as_slice() else {
            continue;
        };
        let Some(symbol_targets) = symbols.get(target_symbol) else {
            continue;
        };
        let target_declarations = symbol_targets.iter().collect::<Vec<_>>();
        let [(target_declaration, _definition_providers)] = target_declarations.as_slice() else {
            continue;
        };
        let Some([call_site]) = sites.calls.get(&anchor).map(Vec::as_slice) else {
            continue;
        };
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
        extraction
            .edges
            .push(compiler_edge(source.clone(), target.clone(), call_site));
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

fn call_site(candidate: &RelationshipCandidate, range: &EvidenceRange) -> CallSite {
    let mut allowed_target_kinds = candidate.constraints.allowed_target_kinds.clone();
    allowed_target_kinds.sort();
    allowed_target_kinds.dedup();
    CallSite {
        candidate_id: candidate.id.clone(),
        source_declaration_id: candidate.source_declaration_id.clone(),
        allowed_target_kinds,
        range: range.clone(),
    }
}

fn anchor_key(range: &EvidenceRange) -> AnchorKey {
    (range.source_file.clone(), range.start_byte, range.end_byte)
}

fn compiler_edge(source: String, target: String, site: &CallSite) -> RawEdgeRecord {
    let rule = "compiler-exact-anchor";
    let attributes = Map::from_iter([
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
        ("language".to_owned(), Value::String("java".to_owned())),
        (
            "extractor".to_owned(),
            Value::String("compass.resolve.java.program".to_owned()),
        ),
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

    fn compiler_anchor(range: &EvidenceRange) -> compass_ir::SourceAnchor {
        compass_ir::SourceAnchor {
            source_file: range.source_file.clone(),
            start_byte: range.start_byte,
            end_byte: range.end_byte,
        }
    }
}
