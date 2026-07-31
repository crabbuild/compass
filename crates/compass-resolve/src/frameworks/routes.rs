use std::collections::{BTreeMap, HashMap, HashSet};

use compass_languages::{
    Extraction, FrameworkLimitError, FrameworkLimits, RawEdgeRecord, RawFrameworkAnchor,
    RawFrameworkFact, RawFrameworkOrigin, RawNodeRecord, RawRouteFact, make_id,
};
use compass_model::code_graph::{
    RouteStage as PublishedRouteStage, RouteStageDetails as PublishedRouteStageDetails,
};
use compass_model::provenance::{
    EvidenceConfidence, EvidenceOrigin, Provenance, ResolutionCandidate, ResolutionState,
    SourceAnchor,
};
use serde_json::{Map, Value};

use super::target_index::{FrameworkTargetIndex, TargetFamily, normalize_reference, terminal_name};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RouteStageRole {
    Middleware,
    Handler,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RouteStage {
    pub position: u32,
    pub role: RouteStageRole,
    pub reference: String,
    pub state: ResolutionState,
    pub target: Option<String>,
    pub candidates: Vec<ResolutionCandidate>,
    pub provenance: Provenance,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedRoute {
    pub route: RawRouteFact,
    pub state: ResolutionState,
    pub stages: Vec<RouteStage>,
    pub candidates: Vec<ResolutionCandidate>,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum FrameworkResolutionError {
    #[error("invalid {framework} route: {detail}")]
    InvalidRoute { framework: String, detail: String },
    #[error(transparent)]
    Limit(#[from] FrameworkLimitError),
    #[error("framework alias expansion limit exceeded: observed {observed}, maximum {maximum}")]
    AliasLimit { observed: usize, maximum: usize },
}

pub fn resolve_routes(
    extraction: &Extraction,
    limits: FrameworkLimits,
) -> Result<Vec<ResolvedRoute>, FrameworkResolutionError> {
    let targets = FrameworkTargetIndex::new(extraction);
    resolve_routes_with_targets(extraction, limits, &targets)
}

pub(super) fn resolve_routes_with_targets(
    extraction: &Extraction,
    limits: FrameworkLimits,
    targets: &FrameworkTargetIndex<'_>,
) -> Result<Vec<ResolvedRoute>, FrameworkResolutionError> {
    validate_fact_limits(extraction, limits)?;
    let aliases = alias_map(extraction, limits)?;
    let expanded = super::python::expand_django_includes(&extraction.framework_facts, limits)?;
    let mut unique = BTreeMap::new();
    for route in expanded {
        route
            .validate()
            .map_err(|detail| FrameworkResolutionError::InvalidRoute {
                framework: route.framework.clone(),
                detail: detail.to_owned(),
            })?;
        let key = (
            route.anchor.source_file.clone(),
            route.framework.clone(),
            route.operation.clone(),
            route.normalized_path.clone(),
            route.declaring_scope.clone(),
            route.anchor.start_byte,
            route.anchor.end_byte,
            route.handler_reference.clone(),
            route.middleware_references.clone(),
            route.origin.as_str(),
            route.rule.clone(),
        );
        unique.entry(key).or_insert(route);
    }

    unique
        .into_values()
        .map(|route| resolve_one_route(route, targets, &aliases, limits))
        .collect()
}

pub fn resolve_and_publish_framework_routes(
    extraction: &mut Extraction,
    limits: FrameworkLimits,
) -> Result<Vec<ResolvedRoute>, FrameworkResolutionError> {
    let resolved = resolve_routes(extraction, limits)?;
    publish_resolved_routes(extraction, &resolved)?;
    Ok(resolved)
}

pub fn publish_resolved_routes(
    extraction: &mut Extraction,
    routes: &[ResolvedRoute],
) -> Result<(), FrameworkResolutionError> {
    let mut existing_nodes = extraction
        .nodes
        .iter()
        .map(|node| node.id.clone())
        .collect::<HashSet<_>>();
    let mut existing_edges = extraction
        .edges
        .iter()
        .map(|edge| {
            (
                edge.source.clone(),
                edge.target.clone(),
                edge.string("relation"),
                edge.string("source_location"),
                edge.string("position"),
                edge.attributes
                    .get("source_anchor")
                    .map(Value::to_string)
                    .unwrap_or_default(),
            )
        })
        .collect::<HashSet<_>>();

    for resolved in routes {
        let route = &resolved.route;
        let route_id = make_id(&[
            "framework-route",
            &route.framework,
            &route.anchor.source_file,
            &route.operation,
            &route.normalized_path,
            &route.declaring_scope,
            &route.anchor.start_byte.to_string(),
            &route.anchor.end_byte.to_string(),
            &route.anchor.start_line.to_string(),
            &route.anchor.start_column.to_string(),
            &route.anchor.end_line.to_string(),
            &route.anchor.end_column.to_string(),
        ]);
        if existing_nodes.insert(route_id.clone()) {
            extraction.nodes.push(RawNodeRecord {
                id: route_id.clone(),
                attributes: route_attributes(resolved),
            });
        }
        for stage in &resolved.stages {
            let Some(target) = stage.target.as_deref() else {
                continue;
            };
            if stage.state != ResolutionState::Exact || !existing_nodes.contains(target) {
                continue;
            }
            let location = format!("L{}", route.anchor.start_line);
            if !existing_edges.insert((
                route_id.clone(),
                target.to_owned(),
                "routes_to".to_owned(),
                location,
                stage.position.to_string(),
                serde_json::to_value(source_anchor(&route.anchor))
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
            )) {
                continue;
            }
            mark_stage_role(&mut extraction.nodes, target, stage.role);
            extraction.edges.push(RawEdgeRecord {
                source: route_id.clone(),
                target: target.to_owned(),
                attributes: route_edge_attributes(resolved, stage),
            });
        }
    }
    Ok(())
}

fn resolve_one_route(
    route: RawRouteFact,
    targets: &FrameworkTargetIndex<'_>,
    aliases: &super::typescript::ImportAliases,
    limits: FrameworkLimits,
) -> Result<ResolvedRoute, FrameworkResolutionError> {
    let mut stages = Vec::new();
    for (position, reference) in route.middleware_references.iter().enumerate() {
        let candidates = resolve_reference(
            reference,
            &route.framework,
            &route.declaring_scope,
            &route.anchor.source_file,
            targets,
            aliases,
            limits,
        )?;
        stages.push(resolved_stage(
            &route,
            u32::try_from(position).unwrap_or(u32::MAX),
            RouteStageRole::Middleware,
            reference,
            candidates,
        ));
    }

    let candidates = resolve_reference(
        &route.handler_reference,
        &route.framework,
        &route.declaring_scope,
        &route.anchor.source_file,
        targets,
        aliases,
        limits,
    )?;
    let state = candidate_state(&candidates);
    stages.push(resolved_stage(
        &route,
        u32::try_from(route.middleware_references.len()).unwrap_or(u32::MAX),
        RouteStageRole::Handler,
        &route.handler_reference,
        candidates.clone(),
    ));
    Ok(ResolvedRoute {
        route,
        state,
        stages,
        candidates,
    })
}

fn resolve_reference(
    reference: &str,
    framework: &str,
    declaring_scope: &str,
    source_file: &str,
    targets: &FrameworkTargetIndex<'_>,
    aliases: &super::typescript::ImportAliases,
    limits: FrameworkLimits,
) -> Result<Vec<ResolutionCandidate>, FrameworkResolutionError> {
    let reference = canonical_framework_reference(framework, reference);
    let reference = normalize_reference(&reference);
    let alias = import_alias(&reference, source_file, aliases);
    let expanded = expand_alias(&reference, alias.map(|(_, alias)| alias));
    let last = terminal_name(&expanded).to_owned();
    let owner = expanded
        .rsplit_once('.')
        .map(|(owner, _)| normalize_reference(owner));
    let scoped = [
        format!("{declaring_scope}::{expanded}"),
        format!("{declaring_scope}.{expanded}"),
    ];
    let alias_export = alias.map(|(suffix, alias)| {
        if alias.imported == "*" {
            terminal_name(suffix.trim_start_matches(['.', ':'])).to_owned()
        } else if suffix.is_empty() {
            alias.imported.clone()
        } else {
            terminal_name(suffix).to_owned()
        }
    });
    let families = [TargetFamily::Route];
    let max = limits.max_candidates;
    let (mut positions, mut candidates_truncated) = targets.by_id(&expanded, &families, max);
    let mut score = 100_u8;
    let mut reason = "exact stable ID";
    if positions.is_empty()
        && let Some((_, alias)) = alias
        && let Some(export) = alias_export.as_deref()
        && alias.module.starts_with('.')
    {
        (positions, candidates_truncated) =
            targets.by_module_terminal(source_file, &alias.module, export, &families, max);
        score = 98;
        reason = "source-module import/export alias";
    }
    if positions.is_empty()
        && let Some(owner) = owner.as_deref()
    {
        (positions, candidates_truncated) = targets.by_owner_terminal(owner, &last, &families, max);
        score = 97;
        reason = "owner-qualified member";
    }
    if positions.is_empty() {
        (positions, candidates_truncated) =
            targets.by_names(std::slice::from_ref(&expanded), &families, max);
        score = 95;
        reason = "exact qualified name";
    }
    if positions.is_empty() {
        let scoped = scoped
            .iter()
            .map(|value| normalize_reference(value))
            .collect::<Vec<_>>();
        (positions, candidates_truncated) = targets.by_names(&scoped, &families, max);
        score = 90;
        reason = "declaring-scope qualified name";
    }
    if positions.is_empty() {
        (positions, candidates_truncated) =
            targets.by_source_terminal(source_file, &last, &families, max);
        score = 90;
        reason = "same-source route target";
    }
    if positions.is_empty() {
        (positions, candidates_truncated) = targets.by_terminal(&last, &families, max);
        score = 70;
        reason = "unique terminal name";
    }
    if positions.is_empty() {
        return Ok(Vec::new());
    }
    let mut candidates = positions
        .into_iter()
        .map(|position| ResolutionCandidate {
            node_id: targets.targets[position].node.id.clone(),
            reason: reason.to_owned(),
            confidence: if score >= 85 {
                EvidenceConfidence::Exact
            } else {
                EvidenceConfidence::Ambiguous
            },
            score: Some(f64::from(score) / 100.0),
            anchor: node_anchor(targets.targets[position].node),
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| left.node_id.cmp(&right.node_id));
    candidates.dedup_by(|left, right| left.node_id == right.node_id);
    if candidates_truncated {
        for candidate in &mut candidates {
            candidate.confidence = EvidenceConfidence::Ambiguous;
            candidate.reason.push_str(" (candidate set truncated)");
        }
    }
    Ok(candidates)
}

fn canonical_framework_reference(framework: &str, reference: &str) -> String {
    match framework {
        "laravel" | "drupal" => super::php::canonical_reference(reference),
        "rails" => super::ruby::canonical_reference(reference),
        "spring" | "play" => super::jvm::canonical_reference(reference),
        "gin" | "chi" | "gorilla" | "axum" | "actix" | "rocket" | "aspnet" | "vapor" => {
            super::native::canonical_reference(reference)
        }
        _ => reference.to_owned(),
    }
}

fn validate_fact_limits(
    extraction: &Extraction,
    limits: FrameworkLimits,
) -> Result<(), FrameworkResolutionError> {
    let mut counts = HashMap::<&str, usize>::new();
    for fact in &extraction.framework_facts {
        let file = match fact {
            RawFrameworkFact::Route(route) => route.anchor.source_file.as_str(),
            RawFrameworkFact::Domain(domain) => domain.anchor.source_file.as_str(),
        };
        *counts.entry(file).or_default() += 1;
    }
    for count in counts.into_values() {
        limits.check_facts(count)?;
    }
    Ok(())
}

fn alias_map(
    extraction: &Extraction,
    limits: FrameworkLimits,
) -> Result<super::typescript::ImportAliases, FrameworkResolutionError> {
    super::typescript::import_alias_map(extraction, limits)
}

fn import_alias<'a>(
    reference: &'a str,
    source_file: &str,
    aliases: &'a super::typescript::ImportAliases,
) -> Option<(&'a str, &'a super::typescript::ImportAlias)> {
    let split = reference.find(['.', ':']).unwrap_or(reference.len());
    let head = &reference[..split];
    aliases
        .get(&(portable_path(source_file), head.to_owned()))
        .map(|alias| (&reference[split..], alias))
}

fn expand_alias(reference: &str, alias: Option<&super::typescript::ImportAlias>) -> String {
    let split = reference.find(['.', ':']).unwrap_or(reference.len());
    alias.map_or_else(
        || reference.to_owned(),
        |alias| {
            let imported = if alias.imported == "*" {
                String::new()
            } else {
                format!(".{}", alias.imported)
            };
            format!("{}{}{}", alias.module, imported, &reference[split..])
        },
    )
}

fn portable_path(path: &str) -> String {
    path.replace('\\', "/")
}

fn resolved_stage(
    route: &RawRouteFact,
    position: u32,
    role: RouteStageRole,
    reference: &str,
    candidates: Vec<ResolutionCandidate>,
) -> RouteStage {
    let state = candidate_state(&candidates);
    let target = if state == ResolutionState::Exact {
        candidates
            .first()
            .map(|candidate| candidate.node_id.clone())
    } else {
        None
    };
    RouteStage {
        position,
        role,
        reference: reference.to_owned(),
        state,
        target,
        provenance: stage_provenance(route, state, &candidates),
        candidates,
    }
}

fn candidate_state(candidates: &[ResolutionCandidate]) -> ResolutionState {
    match candidates {
        [] => ResolutionState::Unresolved,
        [candidate] if candidate.confidence == EvidenceConfidence::Exact => ResolutionState::Exact,
        [_] => ResolutionState::Unresolved,
        _ => ResolutionState::Ambiguous,
    }
}

fn stage_provenance(
    route: &RawRouteFact,
    state: ResolutionState,
    candidates: &[ResolutionCandidate],
) -> Provenance {
    let anchor = source_anchor(&route.anchor);
    let origin = evidence_origin(route.origin);
    Provenance {
        origin,
        extractor: format!("compass.frameworks.{}", route.framework),
        confidence: if origin == EvidenceOrigin::Heuristic {
            EvidenceConfidence::Inferred
        } else {
            match state {
                ResolutionState::Exact => EvidenceConfidence::Exact,
                ResolutionState::Ambiguous => EvidenceConfidence::Ambiguous,
                ResolutionState::Unresolved => EvidenceConfidence::Inferred,
            }
        },
        rule: route.rule.clone(),
        anchors: (origin != EvidenceOrigin::Heuristic)
            .then_some(anchor.clone())
            .into_iter()
            .collect(),
        wiring_site: (origin == EvidenceOrigin::Heuristic).then_some(anchor),
        score: candidates
            .first()
            .and_then(|candidate| candidate.score)
            .filter(|_| candidates.len() == 1),
        candidates: if origin == EvidenceOrigin::Heuristic || state != ResolutionState::Exact {
            candidates.to_vec()
        } else {
            Vec::new()
        },
    }
}

fn route_attributes(resolved: &ResolvedRoute) -> Map<String, Value> {
    let route = &resolved.route;
    let mut attributes = Map::new();
    attributes.insert(
        "label".into(),
        Value::String(format!("{} {}", route.operation, route.normalized_path)),
    );
    attributes.insert(
        "name".into(),
        Value::String(format!("{} {}", route.operation, route.normalized_path)),
    );
    attributes.insert(
        "qualified_name".into(),
        Value::String(format!(
            "{}::{}::{}",
            route.framework, route.operation, route.normalized_path
        )),
    );
    attributes.insert("symbol_kind".into(), Value::String("route".into()));
    attributes.insert("file_type".into(), Value::String("code".into()));
    attributes.insert("framework".into(), Value::String(route.framework.clone()));
    attributes.insert("operation".into(), Value::String(route.operation.clone()));
    attributes.insert("path".into(), Value::String(route.normalized_path.clone()));
    attributes.insert(
        "original_path".into(),
        Value::String(route.raw_path.clone()),
    );
    attributes.insert(
        "declaring_scope".into(),
        Value::String(route.declaring_scope.clone()),
    );
    attributes.insert(
        "resolution".into(),
        Value::String(resolution_name(resolved.state).to_owned()),
    );
    attributes.insert(
        "middleware_count".into(),
        Value::from(route.middleware_references.len() as u64),
    );
    attributes.insert(
        "stages".into(),
        serde_json::to_value(
            resolved
                .stages
                .iter()
                .map(|stage| PublishedRouteStageDetails {
                    stage: match stage.role {
                        RouteStageRole::Middleware => PublishedRouteStage::Middleware,
                        RouteStageRole::Handler => PublishedRouteStage::Handler,
                    },
                    position: stage.position,
                    reference: stage.reference.clone(),
                    resolution: stage.state,
                    target: stage.target.clone(),
                    candidates: stage.candidates.clone(),
                })
                .collect::<Vec<_>>(),
        )
        .unwrap_or(Value::Null),
    );
    add_evidence_attributes(&mut attributes, route, resolved.state, &resolved.candidates);
    attributes
}

fn route_edge_attributes(resolved: &ResolvedRoute, stage: &RouteStage) -> Map<String, Value> {
    let route = &resolved.route;
    let mut attributes = Map::new();
    attributes.insert("relation".into(), Value::String("routes_to".into()));
    attributes.insert(
        "stage".into(),
        Value::String(
            match stage.role {
                RouteStageRole::Middleware => "middleware",
                RouteStageRole::Handler => "handler",
            }
            .to_owned(),
        ),
    );
    attributes.insert("position".into(), Value::from(stage.position));
    attributes.insert("operation".into(), Value::String(route.operation.clone()));
    attributes.insert("weight".into(), Value::from(1.0));
    let evidence_state = match stage.provenance.confidence {
        EvidenceConfidence::Exact => ResolutionState::Exact,
        EvidenceConfidence::Ambiguous => ResolutionState::Ambiguous,
        EvidenceConfidence::Inferred => ResolutionState::Unresolved,
    };
    add_evidence_attributes(&mut attributes, route, evidence_state, &stage.candidates);
    let stage_name = match stage.role {
        RouteStageRole::Middleware => "middleware",
        RouteStageRole::Handler => "handler",
    };
    let rule = route.rule.as_deref().map_or_else(
        || format!("framework-route-stage:{stage_name}:{}", stage.position),
        |rule| format!("{rule}|stage:{stage_name}:{}", stage.position),
    );
    attributes.insert("rule".into(), Value::String(rule));
    attributes
}

fn add_evidence_attributes(
    attributes: &mut Map<String, Value>,
    route: &RawRouteFact,
    state: ResolutionState,
    candidates: &[ResolutionCandidate],
) {
    attributes.insert(
        "source_file".into(),
        Value::String(route.anchor.source_file.clone()),
    );
    attributes.insert(
        "source_location".into(),
        Value::String(format!("L{}", route.anchor.start_line)),
    );
    attributes.insert(
        "source_anchor".into(),
        serde_json::to_value(source_anchor(&route.anchor)).unwrap_or(Value::Null),
    );
    attributes.insert(
        "_origin".into(),
        Value::String(route.origin.as_str().to_owned()),
    );
    attributes.insert(
        "extractor".into(),
        Value::String(format!("compass.frameworks.{}", route.framework)),
    );
    attributes.insert(
        "confidence".into(),
        Value::String(
            match state {
                ResolutionState::Exact => "EXTRACTED",
                ResolutionState::Ambiguous => "AMBIGUOUS",
                ResolutionState::Unresolved => "INFERRED",
            }
            .to_owned(),
        ),
    );
    if let Some(rule) = &route.rule {
        attributes.insert("rule".into(), Value::String(rule.clone()));
    }
    if !candidates.is_empty() {
        attributes.insert(
            "candidates".into(),
            serde_json::to_value(candidates).unwrap_or_else(|_| Value::Array(Vec::new())),
        );
    }
}

fn mark_stage_role(nodes: &mut [RawNodeRecord], target: &str, role: RouteStageRole) {
    let Some(node) = nodes.iter_mut().find(|node| node.id == target) else {
        return;
    };
    let role = match role {
        RouteStageRole::Middleware => "middleware",
        RouteStageRole::Handler => "route_handler",
    };
    let roles = node
        .attributes
        .entry("roles")
        .or_insert_with(|| Value::Array(Vec::new()));
    let Some(roles) = roles.as_array_mut() else {
        return;
    };
    if !roles.iter().any(|item| item.as_str() == Some(role)) {
        roles.push(Value::String(role.to_owned()));
    }
}

fn node_anchor(node: &RawNodeRecord) -> Option<SourceAnchor> {
    let source_file = node
        .attributes
        .get("source_file")
        .and_then(Value::as_str)
        .filter(|source_file| !source_file.trim().is_empty())?;
    let start_line = node
        .attributes
        .get("line_start")
        .and_then(Value::as_u64)
        .and_then(|line| u32::try_from(line).ok())
        .unwrap_or(1);
    Some(SourceAnchor {
        file: source_file.to_owned(),
        start_byte: node
            .attributes
            .get("start_byte")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        end_byte: node
            .attributes
            .get("end_byte")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        start_line,
        start_column: 0,
        end_line: node
            .attributes
            .get("line_end")
            .and_then(Value::as_u64)
            .and_then(|line| u32::try_from(line).ok())
            .unwrap_or(start_line),
        end_column: 0,
    })
}

fn source_anchor(anchor: &RawFrameworkAnchor) -> SourceAnchor {
    SourceAnchor {
        file: anchor.source_file.clone(),
        start_byte: anchor.start_byte,
        end_byte: anchor.end_byte,
        start_line: anchor.start_line,
        start_column: anchor.start_column,
        end_line: anchor.end_line,
        end_column: anchor.end_column,
    }
}

fn evidence_origin(origin: RawFrameworkOrigin) -> EvidenceOrigin {
    match origin {
        RawFrameworkOrigin::Ast => EvidenceOrigin::Ast,
        RawFrameworkOrigin::Config => EvidenceOrigin::Config,
        RawFrameworkOrigin::Convention => EvidenceOrigin::Convention,
        RawFrameworkOrigin::Heuristic => EvidenceOrigin::Heuristic,
    }
}

fn resolution_name(state: ResolutionState) -> &'static str {
    match state {
        ResolutionState::Exact => "exact",
        ResolutionState::Ambiguous => "ambiguous",
        ResolutionState::Unresolved => "unresolved",
    }
}
