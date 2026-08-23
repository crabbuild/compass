use std::path::{Path, PathBuf};

use ahash::AHashSet as HashSet;
use compass_files::FileSetMatcher;

use compass_languages::{
    Extraction, FrameworkLimitError, FrameworkLimits, RawDomainFact, RawEdgeRecord,
    RawFrameworkFact, RawNodeRecord, make_id,
};
use compass_model::provenance::{ResolutionCandidate, ResolutionState, SourceAnchor};
use serde_json::{Map, Value, json};

use super::FrameworkResolutionError;
use super::target_index::{FrameworkTargetIndex, TargetFamily, normalize_reference, terminal_name};

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedDomainFact {
    pub fact: RawDomainFact,
    pub state: ResolutionState,
    pub source_candidates: Vec<ResolutionCandidate>,
    pub target_candidates: Vec<ResolutionCandidate>,
}

pub fn resolve_domains(
    extraction: &Extraction,
    limits: FrameworkLimits,
) -> Result<Vec<ResolvedDomainFact>, FrameworkResolutionError> {
    let target_extraction = super::materialize_universal_framework_targets(extraction);
    let targets = FrameworkTargetIndex::new(&target_extraction);
    resolve_domains_with_targets(&target_extraction, limits, &targets)
}

pub(super) fn resolve_domains_with_targets(
    extraction: &Extraction,
    limits: FrameworkLimits,
    targets: &FrameworkTargetIndex<'_>,
) -> Result<Vec<ResolvedDomainFact>, FrameworkResolutionError> {
    let facts = extraction
        .framework_facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Domain(fact)
                if !matches!(fact.kind.as_str(), "router_mount" | "router_middleware") =>
            {
                Some(fact.clone())
            }
            RawFrameworkFact::Role(role) => Some(role_as_domain(role)),
            RawFrameworkFact::Route(_)
            | RawFrameworkFact::Annotation(_)
            | RawFrameworkFact::Relation(_) => None,
            RawFrameworkFact::Configuration(configuration) => {
                Some(configuration_as_domain(configuration))
            }
            RawFrameworkFact::FileSet(file_set) => Some(file_set_as_domain(file_set)),
            RawFrameworkFact::Domain(_) => None,
        })
        .collect::<Vec<_>>();
    limits.check_facts(facts.len())?;
    let role_count = extraction
        .framework_facts
        .iter()
        .filter(|fact| matches!(fact, RawFrameworkFact::Role(_)))
        .count();
    limits.check_role_facts(role_count)?;
    facts
        .into_iter()
        .map(|fact| resolve_one(&fact, targets, limits))
        .collect()
}

fn role_as_domain(role: &compass_languages::RawFrameworkRoleFact) -> RawDomainFact {
    let mut detail = role.detail.clone();
    if let Some(reference) = role.subject_reference.as_deref() {
        detail.insert(
            "source_reference".to_owned(),
            Value::String(reference.to_owned()),
        );
    }
    detail.insert("role".to_owned(), Value::String(role.role.clone()));
    detail.insert("pack_id".to_owned(), Value::String(role.pack_id.clone()));
    RawDomainFact {
        framework: role.framework.clone(),
        kind: "ui_role".to_owned(),
        name: role.subject_reference.clone().unwrap_or_default(),
        declaring_scope: role.context.clone().unwrap_or_default(),
        anchor: role.anchor.clone(),
        origin: role.origin,
        detail,
    }
}

fn configuration_as_domain(
    configuration: &compass_languages::RawFrameworkConfigurationFact,
) -> RawDomainFact {
    let mut detail = configuration.detail.clone();
    detail.insert(
        "pack_id".to_owned(),
        Value::String(configuration.pack_id.clone()),
    );
    detail.insert(
        "config_id".to_owned(),
        Value::String(configuration.config_id.clone()),
    );
    detail.insert(
        "field".to_owned(),
        Value::String(configuration.field.clone()),
    );
    detail.insert(
        "ordinal".to_owned(),
        Value::Number(configuration.ordinal.into()),
    );
    detail.insert("complete".to_owned(), Value::Bool(configuration.complete));
    if let Some(value) = configuration.value.clone() {
        detail.insert("value".to_owned(), value);
    }
    detail.insert(
        "config_parent_name".to_owned(),
        Value::String(configuration.config_id.clone()),
    );
    RawDomainFact {
        framework: configuration.framework.clone(),
        kind: "framework_configuration_field".to_owned(),
        name: format!(
            "{}::{}::{}",
            configuration.config_id, configuration.ordinal, configuration.field
        ),
        declaring_scope: configuration.config_id.clone(),
        anchor: configuration.anchor.clone(),
        origin: configuration.origin,
        detail,
    }
}

fn file_set_as_domain(file_set: &compass_languages::RawFrameworkFileSetFact) -> RawDomainFact {
    let mut detail = file_set.detail.clone();
    detail.insert(
        "pack_id".to_owned(),
        Value::String(file_set.pack_id.clone()),
    );
    detail.insert(
        "owner_reference".to_owned(),
        Value::String(file_set.owner_reference.clone()),
    );
    detail.insert(
        "patterns".to_owned(),
        Value::Array(
            file_set
                .patterns
                .iter()
                .cloned()
                .map(Value::String)
                .collect(),
        ),
    );
    if !file_set.negative_patterns.is_empty() {
        detail.insert(
            "negative_patterns".to_owned(),
            Value::Array(
                file_set
                    .negative_patterns
                    .iter()
                    .cloned()
                    .map(Value::String)
                    .collect(),
            ),
        );
    }
    detail.insert("eager".to_owned(), Value::Bool(file_set.eager));
    detail.insert("lazy".to_owned(), Value::Bool(file_set.lazy));
    detail.insert("import_mode".to_owned(), Value::Bool(file_set.import_mode));
    detail.insert("query_mode".to_owned(), Value::Bool(file_set.query_mode));
    if let Some(scope) = file_set.package_scope.clone() {
        detail.insert("package_scope".to_owned(), Value::String(scope));
    }
    RawDomainFact {
        framework: file_set.framework.clone(),
        kind: "framework_file_set".to_owned(),
        name: file_set.owner_reference.clone(),
        declaring_scope: file_set.anchor.source_file.clone(),
        anchor: file_set.anchor.clone(),
        origin: file_set.origin,
        detail,
    }
}

pub fn resolve_and_publish_framework_domains(
    extraction: &mut Extraction,
    limits: FrameworkLimits,
) -> Result<Vec<ResolvedDomainFact>, FrameworkResolutionError> {
    let resolved = resolve_domains(extraction, limits)?;
    publish_resolved_domains(extraction, &resolved);
    Ok(resolved)
}

pub fn publish_resolved_domains(extraction: &mut Extraction, resolved: &[ResolvedDomainFact]) {
    publish_resolved_domains_with_root(extraction, resolved, Path::new("."));
}

/// Publish resolved framework domains while retaining the corpus root used by
/// source inventories. File-set resources use this root to turn portable
/// source identities into the absolute candidates required by the bounded
/// matcher. Keeping the root explicit prevents nested projects from being
/// matched as if their project root were the repository root.
pub fn publish_resolved_domains_with_root(
    extraction: &mut Extraction,
    resolved: &[ResolvedDomainFact],
    root: &Path,
) {
    if resolved.is_empty() {
        return;
    }
    let mut nodes = extraction
        .nodes
        .iter()
        .map(|node| node.id.clone())
        .collect::<HashSet<_>>();
    let mut edges = extraction
        .edges
        .iter()
        .map(|edge| {
            (
                edge.source.clone(),
                edge.target.clone(),
                edge.string("relation"),
            )
        })
        .collect::<HashSet<_>>();
    let mut diagnostics = Vec::new();
    let mut deferred_edges = Vec::<(String, String, String, RawDomainFact, ResolutionState)>::new();

    for resolved in resolved {
        let fact = &resolved.fact;
        if fact.kind == "ui_role" {
            let role = fact.detail.get("role").and_then(Value::as_str);
            let valid_role = role.is_some_and(is_ui_role);
            if resolved.state == ResolutionState::Exact
                && valid_role
                && let [candidate] = resolved.source_candidates.as_slice()
                && let Some(node) = extraction
                    .nodes
                    .iter_mut()
                    .find(|node| node.id == candidate.node_id)
            {
                let roles = node
                    .attributes
                    .entry("roles".to_owned())
                    .or_insert_with(|| Value::Array(Vec::new()));
                if let Some(roles) = roles.as_array_mut() {
                    if !roles.iter().any(|value| value.as_str() == role) {
                        roles.push(Value::String(role.unwrap_or_default().to_owned()));
                    }
                    roles.sort_by(|left, right| {
                        left.as_str()
                            .unwrap_or_default()
                            .cmp(right.as_str().unwrap_or_default())
                    });
                    roles.dedup();
                }
            } else {
                diagnostics.push(json!({
                    "kind": if valid_role {
                        "unresolved_ui_role"
                    } else {
                        "invalid_ui_role"
                    },
                    "framework": fact.framework,
                    "role": role,
                    "source": fact.detail.get("source_reference"),
                    "sourceFile": fact.anchor.source_file,
                    "line": fact.anchor.start_line,
                    "resolution": resolution_name(resolved.state),
                    "candidates": resolved.source_candidates,
                }));
            }
            continue;
        }
        if fact.kind == "orm_mapping" {
            if resolved.state == ResolutionState::Exact
                && let ([model], [table]) = (
                    resolved.source_candidates.as_slice(),
                    resolved.target_candidates.as_slice(),
                )
            {
                push_edge(
                    extraction,
                    &mut edges,
                    &model.node_id,
                    &table.node_id,
                    "maps_to",
                    fact,
                    resolved.state,
                );
            } else {
                diagnostics.push(json!({
                    "kind": "unresolved_orm_mapping",
                    "framework": fact.framework,
                    "model": fact.name,
                    "databaseTable": fact.detail.get("database_table"),
                    "source": fact.anchor.source_file,
                    "line": fact.anchor.start_line,
                    "resolution": resolution_name(resolved.state),
                }));
            }
            continue;
        }
        if fact.kind == "framework_decoration" {
            if resolved.state == ResolutionState::Exact
                && let Some(trait_name) = fact.detail.get("trait").and_then(Value::as_str)
            {
                for candidate in &resolved.source_candidates {
                    if let Some(node) = extraction
                        .nodes
                        .iter_mut()
                        .find(|node| node.id == candidate.node_id)
                    {
                        let traits = node
                            .attributes
                            .entry("framework_traits".to_owned())
                            .or_insert_with(|| Value::Array(Vec::new()));
                        if let Some(traits) = traits.as_array_mut()
                            && !traits
                                .iter()
                                .any(|value| value.as_str() == Some(trait_name))
                        {
                            traits.push(Value::String(trait_name.to_owned()));
                            traits.sort_by(|left, right| left.as_str().cmp(&right.as_str()));
                        }
                    }
                }
            } else {
                diagnostics.push(json!({
                    "kind": "unresolved_framework_decoration",
                    "framework": fact.framework,
                    "trait": fact.name,
                    "source": fact.anchor.source_file,
                    "line": fact.anchor.start_line,
                    "resolution": resolution_name(resolved.state),
                }));
            }
            continue;
        }
        if fact.kind == "injection" {
            if resolved.state == ResolutionState::Exact
                && let ([source], [target]) = (
                    resolved.source_candidates.as_slice(),
                    resolved.target_candidates.as_slice(),
                )
            {
                push_edge(
                    extraction,
                    &mut edges,
                    &source.node_id,
                    &target.node_id,
                    "depends_on",
                    fact,
                    resolved.state,
                );
            } else {
                diagnostics.push(json!({
                    "kind": "unresolved_injection",
                    "framework": fact.framework,
                    "source": fact.detail.get("source_reference"),
                    "target": fact.detail.get("target_reference"),
                    "sourceFile": fact.anchor.source_file,
                    "line": fact.anchor.start_line,
                    "resolution": resolution_name(resolved.state),
                }));
            }
            continue;
        }
        if fact.kind == "route_middleware" {
            let middleware_id = route_middleware_id(fact);
            if nodes.insert(middleware_id.clone()) {
                extraction.nodes.push(RawNodeRecord {
                    id: middleware_id,
                    attributes: route_middleware_attributes(fact),
                });
            }
            continue;
        }

        if matches!(
            fact.kind.as_str(),
            "framework_configuration" | "framework_plugin"
        ) {
            let domain_id = domain_id(fact);
            if nodes.insert(domain_id.clone()) {
                extraction.nodes.push(RawNodeRecord {
                    id: domain_id,
                    attributes: domain_attributes(
                        fact,
                        if fact.kind == "framework_plugin" {
                            "component"
                        } else {
                            "config_key"
                        },
                        resolved.state,
                    ),
                });
            }
            continue;
        }

        let Some(symbol_kind) = domain_node_kind(&fact.kind) else {
            diagnostics.push(json!({
                "kind": "unsupported_domain_fact",
                "framework": fact.framework,
                "domainKind": fact.kind,
                "name": fact.name,
            }));
            continue;
        };
        let domain_id = domain_id(fact);
        if nodes.insert(domain_id.clone()) {
            extraction.nodes.push(RawNodeRecord {
                id: domain_id.clone(),
                attributes: domain_attributes(fact, symbol_kind, resolved.state),
            });
        }
        if fact.kind == "framework_configuration_field" {
            deferred_edges.push((
                configuration_parent_id(&fact.framework, &fact.declaring_scope),
                domain_id.clone(),
                "contains".to_owned(),
                fact.clone(),
                resolved.state,
            ));
        } else if fact.kind == "framework_file_set"
            && let Some(source_id) = extraction.nodes.iter().find_map(|node| {
                (node.attributes.get("symbol_kind").and_then(Value::as_str) == Some("file")
                    && node.attributes.get("source_file").and_then(Value::as_str)
                        == Some(fact.anchor.source_file.as_str()))
                .then(|| node.id.clone())
            })
        {
            push_edge(
                extraction,
                &mut edges,
                &source_id,
                &domain_id,
                "contains",
                fact,
                resolved.state,
            );
            publish_file_set_imports(
                extraction,
                &mut edges,
                &source_id,
                fact,
                root,
                &mut diagnostics,
            );
        }
        if resolved.state != ResolutionState::Exact {
            diagnostics.push(json!({
                "kind": "unresolved_domain_handler",
                "framework": fact.framework,
                "domainKind": fact.kind,
                "name": fact.name,
                "source": fact.anchor.source_file,
                "line": fact.anchor.start_line,
                "resolution": resolution_name(resolved.state),
                "candidates": resolved.source_candidates,
            }));
            continue;
        }
        let Some(source) = resolved.source_candidates.first() else {
            continue;
        };
        if fact.kind == "job" {
            push_edge(
                extraction,
                &mut edges,
                &source.node_id,
                &domain_id,
                "schedules",
                fact,
                resolved.state,
            );
            push_edge(
                extraction,
                &mut edges,
                &domain_id,
                &source.node_id,
                "triggers",
                fact,
                resolved.state,
            );
        } else {
            let relationship = fact
                .detail
                .get("relationship")
                .and_then(Value::as_str)
                .unwrap_or("handles");
            push_edge(
                extraction,
                &mut edges,
                &source.node_id,
                &domain_id,
                relationship,
                fact,
                resolved.state,
            );
        }
    }

    for (source, target, relation, fact, state) in deferred_edges {
        if nodes.contains(&source) && nodes.contains(&target) {
            push_edge(
                extraction, &mut edges, &source, &target, &relation, &fact, state,
            );
        }
    }

    if !diagnostics.is_empty()
        && let Some(values) = extraction
            .extensions
            .entry("framework_domain_diagnostics".to_owned())
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
    {
        values.extend(diagnostics);
    }
}

/// Project a statically declared Vite file set onto the already discovered
/// source inventory.  Matching is deliberately collection-scoped: this
/// function never walks the filesystem and therefore cannot escape the
/// discovery/ignore policy owned by `compass-files`.
fn publish_file_set_imports(
    extraction: &mut Extraction,
    edges: &mut HashSet<(String, String, String)>,
    owner_id: &str,
    fact: &RawDomainFact,
    root: &Path,
    diagnostics: &mut Vec<Value>,
) {
    let Some(package_scope) = fact.detail.get("package_scope").and_then(Value::as_str) else {
        diagnostics.push(json!({
            "kind": "file_set_scope_missing",
            "severity": "warning",
            "framework": fact.framework,
            "source": fact.anchor.source_file,
        }));
        return;
    };
    let raw_patterns = string_values(fact.detail.get("patterns"));
    let raw_negative_patterns = string_values(fact.detail.get("negative_patterns"));
    let (patterns, negative_patterns) = if fact.framework == "vite" {
        normalize_vite_file_set_patterns(
            fact,
            root,
            Path::new(package_scope),
            &raw_patterns,
            &raw_negative_patterns,
            diagnostics,
        )
    } else {
        (raw_patterns, raw_negative_patterns)
    };
    if patterns.is_empty() {
        diagnostics.push(json!({
            "kind": "file_set_patterns_empty",
            "severity": "warning",
            "framework": fact.framework,
            "source": fact.anchor.source_file,
        }));
        return;
    }
    let matcher = match FileSetMatcher::new(
        Path::new(package_scope),
        &patterns,
        &negative_patterns,
        FrameworkLimits::DEFAULT.max_glob_patterns,
    ) {
        Ok(matcher) => matcher,
        Err(error) => {
            diagnostics.push(json!({
                "kind": "file_set_match_error",
                "severity": "error",
                "framework": fact.framework,
                "source": fact.anchor.source_file,
                "message": error.to_string(),
            }));
            return;
        }
    };
    let candidates = extraction
        .nodes
        .iter()
        .filter_map(|node| {
            (node.attributes.get("symbol_kind").and_then(Value::as_str) == Some("file"))
                .then(|| {
                    node.attributes
                        .get("source_file")
                        .and_then(Value::as_str)
                        .map(|source_file| {
                            let path = Path::new(source_file);
                            let absolute = if path.is_absolute() {
                                path.to_path_buf()
                            } else {
                                root.join(path)
                            };
                            (node.id.clone(), absolute)
                        })
                })
                .flatten()
        })
        .collect::<Vec<_>>();
    let matched = match matcher.match_paths(
        candidates.iter().map(|(_, path)| path.as_path()),
        FrameworkLimits::DEFAULT.max_glob_matches_per_pattern,
        FrameworkLimits::DEFAULT.max_file_set_edges,
    ) {
        Ok(matched) => matched,
        Err(error) => {
            diagnostics.push(json!({
                "kind": "file_set_match_error",
                "severity": "error",
                "framework": fact.framework,
                "source": fact.anchor.source_file,
                "message": error.to_string(),
            }));
            return;
        }
    };
    for target_path in matched {
        let Some((target_id, _)) = candidates.iter().find(|(_, path)| path == &target_path) else {
            continue;
        };
        push_file_set_edge(extraction, edges, owner_id, target_id, fact);
    }
}

/// Convert Vite's importer-relative/root-relative/alias glob syntax into the
/// package-root-relative patterns accepted by [`FileSetMatcher`]. Vite's
/// `import.meta.glob` is evaluated from the importing module directory;
/// applying the raw pattern to the repository root silently fans out into
/// unrelated workspaces. This normalizer is lexical (it never walks the
/// filesystem), bounded by the already discovered candidate inventory, and
/// rejects escapes from the declared project scope.
fn normalize_vite_file_set_patterns(
    fact: &RawDomainFact,
    root: &Path,
    scope: &Path,
    includes: &[String],
    excludes: &[String],
    diagnostics: &mut Vec<Value>,
) -> (Vec<String>, Vec<String>) {
    let source = Path::new(&fact.anchor.source_file);
    let source = if source.is_absolute() {
        source.to_path_buf()
    } else {
        root.join(source)
    };
    let base = source.parent().unwrap_or(root);
    let aliases = fact
        .detail
        .get("aliases")
        .and_then(Value::as_object)
        .map(|values| {
            let mut values = values
                .iter()
                .filter_map(|(alias, target)| target.as_str().map(|target| (alias, target)))
                .collect::<Vec<_>>();
            values
                .sort_by(|left, right| right.0.len().cmp(&left.0.len()).then(left.0.cmp(right.0)));
            values
        })
        .unwrap_or_default();

    let normalize = |pattern: &str| normalize_one_vite_pattern(pattern, base, scope, &aliases);
    let mut normalized_includes = Vec::with_capacity(includes.len());
    let mut normalized_excludes = Vec::with_capacity(excludes.len());
    for (kind, patterns, output) in [
        ("include", includes, &mut normalized_includes),
        ("exclude", excludes, &mut normalized_excludes),
    ] {
        for pattern in patterns {
            match normalize(pattern) {
                Some(value) => output.push(value),
                None => diagnostics.push(json!({
                    "kind": "file_set_pattern_unresolved",
                    "severity": "warning",
                    "framework": fact.framework,
                    "source": fact.anchor.source_file,
                    "pattern": pattern,
                    "patternKind": kind,
                })),
            }
        }
    }
    normalized_includes.sort();
    normalized_includes.dedup();
    normalized_excludes.sort();
    normalized_excludes.dedup();
    (normalized_includes, normalized_excludes)
}

fn normalize_one_vite_pattern(
    pattern: &str,
    base: &Path,
    scope: &Path,
    aliases: &[(&String, &str)],
) -> Option<String> {
    let pattern = pattern.trim().replace('\\', "/");
    if pattern.is_empty() {
        return None;
    }
    let (pattern_base, suffix) = if let Some(pattern) = pattern.strip_prefix('/') {
        (scope.to_path_buf(), pattern.to_owned())
    } else if let Some((target, suffix)) = aliases.iter().find_map(|(alias, target)| {
        let alias = alias.trim_end_matches("/*");
        pattern
            .strip_prefix(alias)
            .filter(|suffix| suffix.is_empty() || suffix.starts_with('/'))
            .map(|suffix| (*target, suffix))
    }) {
        let target = target.trim().replace('\\', "/");
        let target = target
            .trim_start_matches("./")
            .trim_end_matches("/*")
            .trim_end_matches('/');
        let suffix = suffix.trim_start_matches('/');
        (scope.to_path_buf().join(target), suffix.to_owned())
    } else if pattern.starts_with('.') {
        (base.to_path_buf(), pattern)
    } else {
        // Vite accepts bare project-relative globs in addition to its
        // documented `./` form. Treat them as project-root-relative rather
        // than importer-relative so the behavior remains conservative.
        (scope.to_path_buf(), pattern)
    };
    let candidate = lexical_join(&pattern_base, &suffix)?;
    let relative = candidate.strip_prefix(scope).ok()?;
    let relative = relative.to_string_lossy().replace('\\', "/");
    (!relative.is_empty() && !relative.split('/').any(|part| part == "..")).then_some(relative)
}

fn lexical_join(base: &Path, suffix: &str) -> Option<PathBuf> {
    let mut result = PathBuf::new();
    for component in base.components().chain(Path::new(suffix).components()) {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !result.pop() {
                    return None;
                }
            }
            component => result.push(component.as_os_str()),
        }
    }
    Some(result)
}

fn push_file_set_edge(
    extraction: &mut Extraction,
    edges: &mut HashSet<(String, String, String)>,
    source: &str,
    target: &str,
    fact: &RawDomainFact,
) {
    if !edges.insert((source.to_owned(), target.to_owned(), "imports".to_owned())) {
        return;
    }
    let mut attributes = Map::from_iter([
        ("relation".into(), Value::String("imports".to_owned())),
        (
            "source_file".into(),
            Value::String(fact.anchor.source_file.clone()),
        ),
        (
            "source_location".into(),
            Value::String(format!("L{}", fact.anchor.start_line)),
        ),
        (
            "source_anchor".into(),
            serde_json::to_value(source_anchor(fact)).unwrap_or(Value::Null),
        ),
        (
            "_origin".into(),
            Value::String(fact.origin.as_str().to_owned()),
        ),
        (
            "extractor".into(),
            Value::String(format!("compass.frameworks.{}.file-set", fact.framework)),
        ),
        ("confidence".into(), Value::String("EXTRACTED".to_owned())),
        ("weight".into(), Value::from(1.0)),
    ]);
    attributes.insert("file_set_owner".into(), Value::String(fact.name.clone()));
    if let Some(value) = fact.detail.get("eager") {
        attributes.insert("file_set_eager".into(), value.clone());
    }
    if let Some(value) = fact.detail.get("lazy") {
        attributes.insert("file_set_lazy".into(), value.clone());
    }
    extraction.edges.push(RawEdgeRecord {
        source: source.to_owned(),
        target: target.to_owned(),
        attributes,
    });
}

fn string_values(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn resolve_one(
    fact: &RawDomainFact,
    targets: &FrameworkTargetIndex<'_>,
    limits: FrameworkLimits,
) -> Result<ResolvedDomainFact, FrameworkResolutionError> {
    if matches!(
        fact.kind.as_str(),
        "framework_configuration"
            | "framework_plugin"
            | "framework_configuration_field"
            | "framework_file_set"
    ) {
        return Ok(ResolvedDomainFact {
            fact: fact.clone(),
            state: ResolutionState::Exact,
            source_candidates: Vec::new(),
            target_candidates: Vec::new(),
        });
    }
    if fact.kind == "route_middleware" {
        return Ok(ResolvedDomainFact {
            fact: fact.clone(),
            state: ResolutionState::Exact,
            source_candidates: Vec::new(),
            target_candidates: Vec::new(),
        });
    }
    if fact.kind == "ui_role" {
        let reference = fact
            .detail
            .get("source_reference")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let (nodes, truncated) = targets.exact_node(reference, limits.max_candidates);
        if truncated {
            return Err(FrameworkLimitError {
                limit: "max_candidates",
                maximum: limits.max_candidates,
                observed: limits.max_candidates.saturating_add(1),
            }
            .into());
        }
        let candidates = nodes
            .into_iter()
            .map(|node| ResolutionCandidate {
                node_id: node.id.clone(),
                reason: "exact UI role declaration identity".to_owned(),
                confidence: compass_model::provenance::EvidenceConfidence::Exact,
                score: Some(1.0),
                anchor: node_anchor(node),
            })
            .collect::<Vec<_>>();
        return Ok(ResolvedDomainFact {
            fact: fact.clone(),
            state: single_state(&candidates),
            source_candidates: candidates,
            target_candidates: Vec::new(),
        });
    }
    if fact.kind == "orm_mapping" {
        let model = fact
            .detail
            .get("model_reference")
            .and_then(Value::as_str)
            .unwrap_or(&fact.name);
        let table = fact
            .detail
            .get("database_table")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let schema = fact
            .detail
            .get("database_schema")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let table = if schema.is_empty() {
            table.to_owned()
        } else {
            format!("{schema}.{table}")
        };
        let sources = resolve_reference(
            targets,
            model,
            TargetKind::Type,
            &fact.anchor.source_file,
            limits,
        )?;
        let targets = resolve_reference(
            targets,
            &table,
            TargetKind::DatabaseTable,
            &fact.anchor.source_file,
            limits,
        )?;
        return Ok(ResolvedDomainFact {
            fact: fact.clone(),
            state: pair_state(&sources, &targets),
            source_candidates: sources,
            target_candidates: targets,
        });
    }
    if fact.kind == "injection" {
        let source = fact
            .detail
            .get("source_reference")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let target = fact
            .detail
            .get("target_reference")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let sources = resolve_reference(
            targets,
            source,
            TargetKind::Type,
            &fact.anchor.source_file,
            limits,
        )?;
        let targets = resolve_reference(
            targets,
            target,
            TargetKind::Type,
            &fact.anchor.source_file,
            limits,
        )?;
        return Ok(ResolvedDomainFact {
            fact: fact.clone(),
            state: pair_state(&sources, &targets),
            source_candidates: sources,
            target_candidates: targets,
        });
    }
    let signature_reference = fact
        .detail
        .get("target_signature_qualified")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let qualified_reference = fact
        .detail
        .get("target_qualified_name")
        .or_else(|| fact.detail.get("handler_reference"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut candidates = if signature_reference.is_empty() {
        Vec::new()
    } else {
        resolve_reference(
            targets,
            signature_reference,
            TargetKind::Callable,
            &fact.anchor.source_file,
            limits,
        )?
    };
    if candidates.is_empty() {
        candidates = resolve_reference(
            targets,
            qualified_reference,
            TargetKind::Callable,
            &fact.anchor.source_file,
            limits,
        )?;
    }
    if candidates.is_empty()
        && fact.kind == "bean_definition"
        && fact.detail.get("owner_kind").and_then(Value::as_str) != Some("method")
    {
        candidates = resolve_reference(
            targets,
            qualified_reference,
            TargetKind::Type,
            &fact.anchor.source_file,
            limits,
        )?;
    }
    Ok(ResolvedDomainFact {
        fact: fact.clone(),
        state: single_state(&candidates),
        source_candidates: candidates,
        target_candidates: Vec::new(),
    })
}

#[derive(Clone, Copy)]
enum TargetKind {
    Callable,
    Type,
    DatabaseTable,
}

fn resolve_reference(
    targets: &FrameworkTargetIndex<'_>,
    reference: &str,
    target_kind: TargetKind,
    declaring_source: &str,
    limits: FrameworkLimits,
) -> Result<Vec<ResolutionCandidate>, FrameworkResolutionError> {
    if reference.trim().is_empty() {
        return Ok(Vec::new());
    }
    let expected = normalize_reference(reference);
    let expected_terminal = terminal_name(&expected);
    let owner = expected.rsplit_once('.').map(|(owner, _)| owner);
    let families = [match target_kind {
        TargetKind::Callable => TargetFamily::Callable,
        TargetKind::Type => TargetFamily::Type,
        TargetKind::DatabaseTable => TargetFamily::DatabaseTable,
    }];
    let max = limits.max_candidates;
    let (mut positions, mut truncated) = targets.by_id(&expected, &families, max);
    let mut score = 100_u8;
    if positions.is_empty() {
        (positions, truncated) = targets.by_names(std::slice::from_ref(&expected), &families, max);
    }
    if positions.is_empty()
        && let Some(owner) = owner
    {
        (positions, truncated) =
            targets.by_owner_terminal(owner, expected_terminal, &families, max);
        score = 97;
    }
    if positions.is_empty() {
        (positions, truncated) =
            targets.by_source_terminal(declaring_source, expected_terminal, &families, max);
        score = 90;
    }
    if positions.is_empty() {
        (positions, truncated) = targets.by_terminal(expected_terminal, &families, max);
        score = 70;
    }
    if positions.is_empty() {
        return Ok(Vec::new());
    }
    if truncated {
        return Err(FrameworkLimitError {
            limit: "max_candidates",
            maximum: limits.max_candidates,
            observed: limits.max_candidates.saturating_add(1),
        }
        .into());
    }
    let mut candidates = positions
        .into_iter()
        .map(|position| ResolutionCandidate {
            node_id: targets.targets[position].node.id.clone(),
            reason: if score >= 90 {
                "exact domain reference".to_owned()
            } else {
                "terminal domain reference".to_owned()
            },
            confidence: if score >= 90 {
                compass_model::provenance::EvidenceConfidence::Exact
            } else {
                compass_model::provenance::EvidenceConfidence::Ambiguous
            },
            score: Some(f64::from(score) / 100.0),
            anchor: node_anchor(targets.targets[position].node),
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| left.node_id.cmp(&right.node_id));
    candidates.dedup_by(|left, right| left.node_id == right.node_id);
    Ok(candidates)
}

fn domain_node_kind(kind: &str) -> Option<&'static str> {
    match kind {
        "event" => Some("event"),
        "message" => Some("message"),
        "topic" => Some("topic"),
        "queue" => Some("queue"),
        "job" => Some("job"),
        "bean_definition" => Some("component"),
        "framework_configuration" => Some("config_key"),
        "framework_plugin" => Some("component"),
        "framework_configuration_field" => Some("config_key"),
        "framework_file_set" => Some("resource"),
        _ => None,
    }
}

fn is_ui_role(role: &str) -> bool {
    matches!(
        role,
        "ui_component"
            | "hook"
            | "client_boundary"
            | "client_component"
            | "server_component"
            | "server_function"
            | "data_loader"
    )
}

fn domain_id(fact: &RawDomainFact) -> String {
    make_id(&[
        "framework-domain",
        &fact.framework,
        &fact.kind,
        fact.detail
            .get("transport")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        &fact.name,
        &fact.declaring_scope,
    ])
}

fn route_middleware_id(fact: &RawDomainFact) -> String {
    make_id(&[
        "framework-route-middleware",
        &fact.framework,
        &fact.anchor.source_file,
        &fact.name,
    ])
}

fn route_middleware_attributes(fact: &RawDomainFact) -> Map<String, Value> {
    Map::from_iter([
        ("label".into(), Value::String(fact.name.clone())),
        ("name".into(), Value::String(fact.name.clone())),
        (
            "qualified_name".into(),
            Value::String(format!("{}::middleware::{}", fact.framework, fact.name)),
        ),
        ("symbol_kind".into(), Value::String("component".to_owned())),
        ("file_type".into(), Value::String("code".to_owned())),
        (
            "component_type".into(),
            Value::String("route_middleware".to_owned()),
        ),
        (
            "roles".into(),
            Value::Array(vec![Value::String("middleware".to_owned())]),
        ),
        ("framework".into(), Value::String(fact.framework.clone())),
        (
            "declaring_scope".into(),
            Value::String(fact.declaring_scope.clone()),
        ),
        (
            "source_file".into(),
            Value::String(fact.anchor.source_file.clone()),
        ),
        (
            "source_location".into(),
            Value::String(format!("L{}", fact.anchor.start_line)),
        ),
        (
            "source_anchor".into(),
            serde_json::to_value(source_anchor(fact)).unwrap_or(Value::Null),
        ),
        (
            "_origin".into(),
            Value::String(fact.origin.as_str().to_owned()),
        ),
        (
            "extractor".into(),
            Value::String(format!("compass.frameworks.{}.domain", fact.framework)),
        ),
        ("confidence".into(), Value::String("EXTRACTED".to_owned())),
        (
            "rule".into(),
            Value::String("route-middleware-file-convention".to_owned()),
        ),
    ])
}

fn domain_attributes(
    fact: &RawDomainFact,
    symbol_kind: &str,
    state: ResolutionState,
) -> Map<String, Value> {
    let mut attributes = Map::from_iter([
        ("label".into(), Value::String(fact.name.clone())),
        ("name".into(), Value::String(fact.name.clone())),
        (
            "qualified_name".into(),
            Value::String(format!("{}::{}::{}", fact.framework, fact.kind, fact.name)),
        ),
        ("symbol_kind".into(), Value::String(symbol_kind.to_owned())),
        ("file_type".into(), Value::String("code".to_owned())),
        ("framework".into(), Value::String(fact.framework.clone())),
        (
            "declaring_scope".into(),
            Value::String(fact.declaring_scope.clone()),
        ),
        (
            "source_file".into(),
            Value::String(fact.anchor.source_file.clone()),
        ),
        (
            "source_location".into(),
            Value::String(format!("L{}", fact.anchor.start_line)),
        ),
        (
            "source_anchor".into(),
            serde_json::to_value(source_anchor(fact)).unwrap_or(Value::Null),
        ),
        (
            "_origin".into(),
            Value::String(fact.origin.as_str().to_owned()),
        ),
        (
            "extractor".into(),
            Value::String(format!("compass.frameworks.{}.domain", fact.framework)),
        ),
        (
            "resolution".into(),
            Value::String(resolution_name(state).to_owned()),
        ),
    ]);
    if matches!(
        fact.kind.as_str(),
        "framework_configuration"
            | "framework_plugin"
            | "framework_configuration_field"
            | "framework_file_set"
    ) {
        attributes.insert("file_type".into(), Value::String("code".to_owned()));
        attributes.insert("component_type".into(), Value::String(fact.kind.clone()));
        for key in [
            "configuration_keys",
            "aliases",
            "aliases_ordered",
            "plugins",
            "route_roots",
            "config_id",
            "field",
            "ordinal",
            "complete",
            "value",
            "pack_id",
            "owner_reference",
            "patterns",
            "negative_patterns",
            "eager",
            "lazy",
            "import_mode",
            "query_mode",
            "package_scope",
            "options",
            "callee",
        ] {
            if let Some(value) = fact.detail.get(key).cloned() {
                attributes.insert(key.to_owned(), value);
            }
        }
        if symbol_kind == "config_key" {
            attributes.insert("format".to_owned(), Value::String("framework".to_owned()));
            attributes.insert("key_path".to_owned(), Value::String(fact.name.clone()));
        }
        if fact.kind == "framework_file_set" {
            attributes.insert(
                "resource_kind".to_owned(),
                Value::String("framework_file_set".to_owned()),
            );
        }
        return attributes;
    }
    if symbol_kind == "component" {
        attributes.insert(
            "component_type".into(),
            fact.detail
                .get("bean_kind")
                .cloned()
                .unwrap_or_else(|| Value::String(fact.kind.clone())),
        );
    }
    if symbol_kind == "job" {
        for key in ["schedule", "queue"] {
            if let Some(value) = fact.detail.get(key).cloned() {
                attributes.insert(key.to_owned(), value);
            }
        }
    } else {
        attributes.insert(
            "transport".into(),
            fact.detail
                .get("transport")
                .cloned()
                .unwrap_or_else(|| Value::String(fact.framework.clone())),
        );
        attributes.insert(
            "subject".into(),
            fact.detail
                .get("subject")
                .cloned()
                .unwrap_or_else(|| Value::String(fact.name.clone())),
        );
    }
    attributes
}

fn configuration_parent_id(framework: &str, config_id: &str) -> String {
    make_id(&[
        "framework-domain",
        framework,
        "framework_configuration",
        "",
        config_id,
        config_id,
    ])
}

fn push_edge(
    extraction: &mut Extraction,
    existing: &mut HashSet<(String, String, String)>,
    source: &str,
    target: &str,
    relation: &str,
    fact: &RawDomainFact,
    state: ResolutionState,
) {
    if !existing.insert((source.to_owned(), target.to_owned(), relation.to_owned())) {
        return;
    }
    extraction.edges.push(RawEdgeRecord {
        source: source.to_owned(),
        target: target.to_owned(),
        attributes: Map::from_iter([
            ("relation".into(), Value::String(relation.to_owned())),
            (
                "source_file".into(),
                Value::String(fact.anchor.source_file.clone()),
            ),
            (
                "source_location".into(),
                Value::String(format!("L{}", fact.anchor.start_line)),
            ),
            (
                "source_anchor".into(),
                serde_json::to_value(source_anchor(fact)).unwrap_or(Value::Null),
            ),
            (
                "_origin".into(),
                Value::String(fact.origin.as_str().to_owned()),
            ),
            (
                "extractor".into(),
                Value::String(format!("compass.frameworks.{}.domain", fact.framework)),
            ),
            (
                "confidence".into(),
                Value::String(
                    if state == ResolutionState::Exact {
                        "EXTRACTED"
                    } else {
                        "AMBIGUOUS"
                    }
                    .to_owned(),
                ),
            ),
            ("weight".into(), Value::from(1.0)),
        ]),
    });
}

fn source_anchor(fact: &RawDomainFact) -> SourceAnchor {
    SourceAnchor {
        file: fact.anchor.source_file.clone(),
        start_byte: fact.anchor.start_byte,
        end_byte: fact.anchor.end_byte,
        start_line: fact.anchor.start_line,
        start_column: fact.anchor.start_column,
        end_line: fact.anchor.end_line,
        end_column: fact.anchor.end_column,
    }
}

fn node_anchor(node: &RawNodeRecord) -> Option<SourceAnchor> {
    let file = node.attributes.get("source_file").and_then(Value::as_str)?;
    let line = node
        .attributes
        .get("line_start")
        .and_then(Value::as_u64)
        .and_then(|line| u32::try_from(line).ok())
        .unwrap_or(1);
    Some(SourceAnchor {
        file: file.to_owned(),
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
        start_line: line,
        start_column: 0,
        end_line: line,
        end_column: 0,
    })
}

fn single_state(candidates: &[ResolutionCandidate]) -> ResolutionState {
    match candidates {
        [] => ResolutionState::Unresolved,
        [candidate]
            if candidate.confidence == compass_model::provenance::EvidenceConfidence::Exact =>
        {
            ResolutionState::Exact
        }
        _ => ResolutionState::Ambiguous,
    }
}

fn pair_state(left: &[ResolutionCandidate], right: &[ResolutionCandidate]) -> ResolutionState {
    let left = single_state(left);
    let right = single_state(right);
    if left == ResolutionState::Exact && right == ResolutionState::Exact {
        ResolutionState::Exact
    } else if left == ResolutionState::Ambiguous || right == ResolutionState::Ambiguous {
        ResolutionState::Ambiguous
    } else {
        ResolutionState::Unresolved
    }
}

fn resolution_name(state: ResolutionState) -> &'static str {
    match state {
        ResolutionState::Exact => "exact",
        ResolutionState::Ambiguous => "ambiguous",
        ResolutionState::Unresolved => "unresolved",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use compass_languages::{RawFrameworkAnchor, RawFrameworkOrigin};

    fn vite_file_set_fact(source_file: &str) -> RawDomainFact {
        RawDomainFact {
            framework: "vite".to_owned(),
            kind: "framework_file_set".to_owned(),
            name: "import.meta.glob".to_owned(),
            declaring_scope: source_file.to_owned(),
            anchor: RawFrameworkAnchor {
                source_file: source_file.to_owned(),
                start_byte: 10,
                end_byte: 28,
                start_line: 1,
                start_column: 1,
                end_line: 1,
                end_column: 19,
            },
            origin: RawFrameworkOrigin::Ast,
            detail: Map::new(),
        }
    }

    #[test]
    fn vite_file_set_patterns_are_importer_relative_and_scope_bounded() {
        let root = Path::new("/workspace/repository");
        let scope = root.join("packages/app");
        let fact = vite_file_set_fact("packages/app/src/config/vite.config.ts");
        let mut diagnostics = Vec::new();

        let (includes, excludes) = normalize_vite_file_set_patterns(
            &fact,
            root,
            &scope,
            &[
                "./fixtures/*.tsx".to_owned(),
                "../shared/*.ts".to_owned(),
                "/public/*.svg".to_owned(),
                "../../../outside/*.ts".to_owned(),
            ],
            &["./fixtures/ignored.tsx".to_owned()],
            &mut diagnostics,
        );

        assert_eq!(
            includes,
            vec![
                "public/*.svg".to_owned(),
                "src/config/fixtures/*.tsx".to_owned(),
                "src/shared/*.ts".to_owned(),
            ]
        );
        assert_eq!(excludes, vec!["src/config/fixtures/ignored.tsx".to_owned()]);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].get("pattern").and_then(Value::as_str),
            Some("../../../outside/*.ts")
        );
        assert_eq!(
            diagnostics[0].get("kind").and_then(Value::as_str),
            Some("file_set_pattern_unresolved")
        );
    }

    #[test]
    fn vite_file_set_aliases_are_longest_match_and_scope_bounded() {
        let root = Path::new("/workspace/repository");
        let scope = root.join("packages/app");
        let mut fact = vite_file_set_fact("packages/app/src/config/vite.config.ts");
        fact.detail.insert(
            "aliases".to_owned(),
            json!({
                "@app/*": "./src/*",
                "@app/components/*": "./src/components/*"
            }),
        );
        let mut diagnostics = Vec::new();

        let (includes, _) = normalize_vite_file_set_patterns(
            &fact,
            root,
            &scope,
            &[
                "@app/components/**/*.tsx".to_owned(),
                "@app/utils/**/*.ts".to_owned(),
            ],
            &[],
            &mut diagnostics,
        );

        assert_eq!(
            includes,
            vec![
                "src/components/**/*.tsx".to_owned(),
                "src/utils/**/*.ts".to_owned(),
            ]
        );
        assert!(diagnostics.is_empty());
    }
}
