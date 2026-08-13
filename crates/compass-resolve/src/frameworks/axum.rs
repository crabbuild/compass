use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use compass_languages::{FrameworkLimitError, FrameworkLimits, RawFrameworkFact, RawRouteFact};
use serde_json::Value;

use super::routes::FrameworkResolutionError;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct RouterKey {
    source_file: String,
    owner: String,
}

#[derive(Clone)]
struct RouterMount {
    parent: RouterKey,
    prefix: String,
    anchor: Value,
    anchor_end: u64,
}

#[derive(Clone)]
struct RouterMiddleware {
    reference: String,
    anchor: Value,
    applies_through: u64,
}

pub(super) fn expand_router_mounts(
    facts: &[RawFrameworkFact],
    routes: Vec<RawRouteFact>,
    limits: FrameworkLimits,
) -> Result<Vec<RawRouteFact>, FrameworkResolutionError> {
    let mount_facts = facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Domain(domain)
                if domain.framework == "axum" && domain.kind == "router_mount" =>
            {
                Some(domain)
            }
            RawFrameworkFact::Route(_)
            | RawFrameworkFact::Domain(_)
            | RawFrameworkFact::Annotation(_) => None,
        })
        .collect::<Vec<_>>();
    let has_middleware = facts.iter().any(|fact| {
        matches!(
            fact,
            RawFrameworkFact::Domain(domain)
                if domain.framework == "axum" && domain.kind == "router_middleware"
        )
    });
    if mount_facts.is_empty() && !has_middleware {
        return Ok(routes);
    }

    let mut owners = BTreeSet::new();
    for route in &routes {
        if route.framework != "axum" {
            continue;
        }
        if let Some(owner) = route.detail.get("router_owner").and_then(Value::as_str) {
            owners.insert(RouterKey {
                source_file: normalized_source(&route.anchor.source_file),
                owner: owner.to_owned(),
            });
        }
    }
    for fact in &mount_facts {
        if let Some(parent) = fact.detail.get("parent_router").and_then(Value::as_str) {
            owners.insert(RouterKey {
                source_file: normalized_source(&fact.anchor.source_file),
                owner: parent.to_owned(),
            });
        }
        if let Some(target) = fact.detail.get("target_router").and_then(Value::as_str) {
            owners.insert(RouterKey {
                source_file: normalized_source(&fact.anchor.source_file),
                owner: target.to_owned(),
            });
        }
    }
    for fact in facts {
        let RawFrameworkFact::Domain(domain) = fact else {
            continue;
        };
        if domain.framework != "axum" || domain.kind != "router_middleware" {
            continue;
        }
        if let Some(parent) = domain.detail.get("parent_router").and_then(Value::as_str) {
            owners.insert(RouterKey {
                source_file: normalized_source(&domain.anchor.source_file),
                owner: parent.to_owned(),
            });
        }
    }

    let mut incoming = BTreeMap::<RouterKey, Vec<RouterMount>>::new();
    for fact in mount_facts {
        let Some(parent_owner) = fact.detail.get("parent_router").and_then(Value::as_str) else {
            continue;
        };
        let source_file = normalized_source(&fact.anchor.source_file);
        let parent = RouterKey {
            source_file: source_file.clone(),
            owner: parent_owner.to_owned(),
        };
        let target = if let Some(owner) = fact.detail.get("target_router").and_then(Value::as_str) {
            Some(RouterKey {
                source_file: source_file.clone(),
                owner: owner.to_owned(),
            })
        } else {
            fact.detail
                .get("target_reference")
                .and_then(Value::as_str)
                .and_then(|reference| resolve_router_reference(&source_file, reference, &owners))
        };
        let Some(target) = target.filter(|target| owners.contains(target)) else {
            continue;
        };
        incoming.entry(target).or_default().push(RouterMount {
            parent,
            prefix: fact
                .detail
                .get("mount_prefix")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            anchor: serde_json::to_value(&fact.anchor).unwrap_or(Value::Null),
            anchor_end: if fact.detail.get("mount_operation").and_then(Value::as_str)
                == Some("alias")
            {
                0
            } else {
                fact.anchor.end_byte
            },
        });
    }
    for mounts in incoming.values_mut() {
        mounts.sort_by(|left, right| {
            (
                &left.parent.source_file,
                &left.parent.owner,
                &left.prefix,
                left.anchor.to_string(),
            )
                .cmp(&(
                    &right.parent.source_file,
                    &right.parent.owner,
                    &right.prefix,
                    right.anchor.to_string(),
                ))
        });
        mounts.dedup_by(|left, right| {
            left.parent == right.parent
                && left.prefix == right.prefix
                && left.anchor == right.anchor
        });
    }
    let mut middleware = BTreeMap::<RouterKey, Vec<RouterMiddleware>>::new();
    for fact in facts {
        let RawFrameworkFact::Domain(domain) = fact else {
            continue;
        };
        if domain.framework != "axum" || domain.kind != "router_middleware" {
            continue;
        }
        let (Some(owner), Some(reference)) = (
            domain.detail.get("parent_router").and_then(Value::as_str),
            domain
                .detail
                .get("middleware_reference")
                .and_then(Value::as_str),
        ) else {
            continue;
        };
        middleware
            .entry(RouterKey {
                source_file: normalized_source(&domain.anchor.source_file),
                owner: owner.to_owned(),
            })
            .or_default()
            .push(RouterMiddleware {
                reference: reference.to_owned(),
                anchor: serde_json::to_value(&domain.anchor).unwrap_or(Value::Null),
                applies_through: domain.anchor.end_byte,
            });
    }
    for values in middleware.values_mut() {
        values.sort_by(|left, right| {
            (left.anchor.to_string(), &left.reference)
                .cmp(&(right.anchor.to_string(), &right.reference))
        });
        values.dedup_by(|left, right| {
            left.reference == right.reference && left.anchor == right.anchor
        });
    }

    let mut output = Vec::new();
    let mut expansions = 0_usize;
    for route in routes {
        if route.framework != "axum" {
            output.push(route);
            continue;
        }
        let Some(owner) = route.detail.get("router_owner").and_then(Value::as_str) else {
            output.push(route);
            continue;
        };
        let key = RouterKey {
            source_file: normalized_source(&route.anchor.source_file),
            owner: owner.to_owned(),
        };
        let registration_end = route.anchor.end_byte;
        let route = with_router_middleware(route, &key, &middleware, registration_end, false);
        if has_mount_cycle(&key, &incoming, &mut BTreeSet::new(), &mut BTreeSet::new()) {
            output.push(route);
            continue;
        }
        let mut visiting = BTreeSet::new();
        expand_route(
            route,
            key,
            &incoming,
            &middleware,
            limits,
            0,
            &mut expansions,
            &mut visiting,
            &mut output,
        )?;
    }
    Ok(output)
}

fn has_mount_cycle(
    owner: &RouterKey,
    incoming: &BTreeMap<RouterKey, Vec<RouterMount>>,
    visited: &mut BTreeSet<RouterKey>,
    active: &mut BTreeSet<RouterKey>,
) -> bool {
    if active.contains(owner) {
        return true;
    }
    if !visited.insert(owner.clone()) {
        return false;
    }
    active.insert(owner.clone());
    let cyclic = incoming.get(owner).is_some_and(|mounts| {
        mounts
            .iter()
            .any(|mount| has_mount_cycle(&mount.parent, incoming, visited, active))
    });
    active.remove(owner);
    cyclic
}

#[allow(clippy::too_many_arguments)]
fn expand_route(
    route: RawRouteFact,
    owner: RouterKey,
    incoming: &BTreeMap<RouterKey, Vec<RouterMount>>,
    middleware: &BTreeMap<RouterKey, Vec<RouterMiddleware>>,
    limits: FrameworkLimits,
    depth: usize,
    expansions: &mut usize,
    visiting: &mut BTreeSet<RouterKey>,
    output: &mut Vec<RawRouteFact>,
) -> Result<(), FrameworkResolutionError> {
    if depth >= limits.max_include_depth {
        return Err(FrameworkLimitError {
            limit: "max_include_depth",
            maximum: limits.max_include_depth,
            observed: depth + 1,
        }
        .into());
    }
    if !visiting.insert(owner.clone()) {
        output.push(route);
        return Ok(());
    }
    let Some(mounts) = incoming.get(&owner) else {
        visiting.remove(&owner);
        output.push(route);
        return Ok(());
    };
    for mount in mounts {
        *expansions = expansions.saturating_add(1);
        if *expansions > limits.max_alias_expansions {
            return Err(FrameworkLimitError {
                limit: "max_alias_expansions",
                maximum: limits.max_alias_expansions,
                observed: *expansions,
            }
            .into());
        }
        let mut expanded = route.clone();
        expanded.normalized_path = compose_paths(&mount.prefix, &expanded.normalized_path);
        expanded.raw_path = compose_paths(&mount.prefix, &expanded.raw_path);
        let chain = expanded
            .detail
            .entry("router_mounts".to_owned())
            .or_insert_with(|| Value::Array(Vec::new()));
        if let Some(chain) = chain.as_array_mut() {
            chain.push(serde_json::json!({
                "prefix": mount.prefix,
                "anchor": mount.anchor,
            }));
        }
        let expanded =
            with_router_middleware(expanded, &mount.parent, middleware, mount.anchor_end, true);
        expand_route(
            expanded,
            mount.parent.clone(),
            incoming,
            middleware,
            limits,
            depth + 1,
            expansions,
            visiting,
            output,
        )?;
    }
    visiting.remove(&owner);
    Ok(())
}

fn with_router_middleware(
    mut route: RawRouteFact,
    owner: &RouterKey,
    middleware: &BTreeMap<RouterKey, Vec<RouterMiddleware>>,
    registration_end: u64,
    prepend: bool,
) -> RawRouteFact {
    let Some(values) = middleware.get(owner) else {
        return route;
    };
    let references = values
        .iter()
        .filter(|middleware| registration_end <= middleware.applies_through)
        .map(|middleware| middleware.reference.clone())
        .collect::<Vec<_>>();
    if references.is_empty() {
        return route;
    }
    if prepend {
        let mut combined = references;
        combined.extend(route.middleware_references);
        route.middleware_references = combined;
    } else {
        route.middleware_references.extend(references);
    }
    let anchors = route
        .detail
        .entry("router_middleware".to_owned())
        .or_insert_with(|| Value::Array(Vec::new()));
    if let Some(anchors) = anchors.as_array_mut() {
        anchors.extend(
            values
                .iter()
                .filter(|middleware| registration_end <= middleware.applies_through)
                .map(|middleware| {
                    serde_json::json!({
                        "reference": middleware.reference,
                        "anchor": middleware.anchor,
                    })
                }),
        );
    }
    route
}

fn resolve_router_reference(
    source_file: &str,
    reference: &str,
    owners: &BTreeSet<RouterKey>,
) -> Option<RouterKey> {
    let mut segments = reference
        .split("::")
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let function = segments.pop()?;
    let owner = format!("fn:{function}");
    if segments.is_empty() {
        let key = RouterKey {
            source_file: source_file.to_owned(),
            owner,
        };
        return owners.contains(&key).then_some(key);
    }
    let candidate_files = rust_module_source_candidates(source_file, &segments);
    let matches = owners
        .iter()
        .filter(|candidate| {
            candidate.owner == owner && candidate_files.contains(&candidate.source_file)
        })
        .cloned()
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [matched] => Some(matched.clone()),
        [] | [_, ..] => None,
    }
}

fn rust_module_source_candidates(source_file: &str, modules: &[&str]) -> BTreeSet<String> {
    let source = Path::new(source_file);
    let parent = source.parent().unwrap_or_else(|| Path::new(""));
    let mut candidates = BTreeSet::new();
    let mut relative = modules;
    let mut bases = Vec::new();
    if modules.first().copied() == Some("crate") {
        relative = &modules[1..];
        bases.push(crate_source_root(source));
    } else {
        bases.push(parent.to_path_buf());
        if source.file_name().and_then(|name| name.to_str()) != Some("mod.rs")
            && let Some(stem) = source.file_stem()
        {
            bases.push(parent.join(stem));
        }
    }
    for base in bases {
        let mut module = base;
        for segment in relative {
            match *segment {
                "self" => {}
                "super" => {
                    module.pop();
                }
                segment => module.push(segment),
            }
        }
        let mut file = module.clone();
        file.set_extension("rs");
        candidates.insert(normalized_source(&file.to_string_lossy()));
        candidates.insert(normalized_source(&module.join("mod.rs").to_string_lossy()));
    }
    candidates
}

fn crate_source_root(source: &Path) -> PathBuf {
    let components = source.components().collect::<Vec<_>>();
    let Some(index) = components
        .iter()
        .rposition(|component| component.as_os_str() == "src")
    else {
        return source
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .to_path_buf();
    };
    components[..=index].iter().collect()
}

fn compose_paths(parent: &str, child: &str) -> String {
    let mut result = format!("{}/{}", parent.trim_matches('/'), child.trim_matches('/'));
    if !result.starts_with('/') {
        result.insert(0, '/');
    }
    while result.contains("//") {
        result = result.replace("//", "/");
    }
    if result.len() > 1 {
        result = result.trim_end_matches('/').to_owned();
    }
    result
}

fn normalized_source(source: &str) -> String {
    source.replace('\\', "/")
}
