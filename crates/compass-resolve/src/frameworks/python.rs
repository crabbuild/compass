use ahash::{AHashMap as HashMap, AHashSet as HashSet};

use compass_languages::{FrameworkLimitError, FrameworkLimits, RawFrameworkFact, RawRouteFact};
use serde_json::Value;

use super::routes::FrameworkResolutionError;

pub(super) fn expand_routes(
    facts: &[RawFrameworkFact],
    routes: Vec<RawRouteFact>,
    limits: FrameworkLimits,
) -> Result<Vec<RawRouteFact>, FrameworkResolutionError> {
    let routes = expand_django_includes(routes, limits)?;
    expand_router_mounts(facts, routes, limits)
}

fn expand_django_includes(
    routes: Vec<RawRouteFact>,
    limits: FrameworkLimits,
) -> Result<Vec<RawRouteFact>, FrameworkResolutionError> {
    let mut by_scope = HashMap::<String, Vec<usize>>::new();
    for (index, route) in routes.iter().enumerate() {
        if route.framework == "django" {
            for key in scope_keys(&route.declaring_scope) {
                by_scope.entry(key).or_default().push(index);
            }
        }
    }
    let included_scopes = routes
        .iter()
        .filter_map(include_target)
        .flat_map(|target| include_scope_candidates(&target))
        .collect::<HashSet<_>>();

    let mut output = Vec::new();
    for (index, route) in routes.iter().enumerate() {
        if route.framework == "django" && include_target(route).is_some() {
            let mut visiting = HashSet::new();
            expand_one(
                index,
                &routes,
                &by_scope,
                limits,
                0,
                &mut visiting,
                &mut output,
            )?;
        } else if route.framework != "django"
            || !scope_keys(&route.declaring_scope)
                .iter()
                .any(|scope| included_scopes.contains(scope))
        {
            output.push(route.clone());
        }
    }
    let mut output_per_file = HashMap::new();
    for route in &output {
        *output_per_file
            .entry(route.anchor.source_file.as_str())
            .or_insert(0_usize) += 1;
    }
    for count in output_per_file.into_values() {
        limits.check_facts(count)?;
    }
    Ok(output)
}

fn expand_router_mounts(
    facts: &[RawFrameworkFact],
    routes: Vec<RawRouteFact>,
    limits: FrameworkLimits,
) -> Result<Vec<RawRouteFact>, FrameworkResolutionError> {
    let mounts = facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Domain(domain) if domain.kind == "router_mount" => Some(domain),
            _ => None,
        })
        .collect::<Vec<_>>();
    if mounts.is_empty() {
        return Ok(routes);
    }
    let mut output = Vec::new();
    for route in routes {
        if !matches!(route.framework.as_str(), "fastapi" | "flask")
            || route
                .detail
                .get("mount_prefix")
                .and_then(Value::as_str)
                .is_some_and(|prefix| !prefix.is_empty())
        {
            output.push(route);
            continue;
        }
        let receiver = route
            .detail
            .get("receiver")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let route_scope = normalize_mount_scope(&route.declaring_scope);
        let applicable = mounts
            .iter()
            .filter(|mount| mount.framework == route.framework)
            .filter(|mount| {
                mount.detail.get("target_receiver").and_then(Value::as_str) == Some(receiver)
            })
            .filter(|mount| {
                let target_module = mount
                    .detail
                    .get("target_module")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if target_module.is_empty() {
                    // An unqualified receiver is only safe within the mount
                    // module itself. Applying it to every same-named router
                    // in the repository would create a false cross-module
                    // binding when two modules both use `router`.
                    mount.declaring_scope == route_scope
                } else {
                    route_scope == normalize_mount_scope(target_module)
                        || route_scope.ends_with(&format!(".{target_module}"))
                        || mount.declaring_scope == route_scope
                }
            })
            .collect::<Vec<_>>();
        if applicable.is_empty() {
            output.push(route);
            continue;
        }
        for mount in applicable {
            let Some(prefix) = mount.detail.get("mount_prefix").and_then(Value::as_str) else {
                continue;
            };
            let mut expanded = route.clone();
            expanded.normalized_path = compose_paths(prefix, &route.normalized_path);
            expanded.raw_path = format!(
                "{}{}",
                prefix.trim_end_matches('/'),
                route.raw_path.trim_start_matches('/')
            );
            expanded
                .detail
                .insert("mount_prefix".into(), Value::String(prefix.to_owned()));
            expanded.detail.insert(
                "mount_anchor".into(),
                serde_json::to_value(&mount.anchor).unwrap_or(Value::Null),
            );
            output.push(expanded);
        }
    }
    let mut per_file = HashMap::new();
    for route in &output {
        *per_file
            .entry(route.anchor.source_file.as_str())
            .or_insert(0_usize) += 1;
    }
    for count in per_file.into_values() {
        limits.check_facts(count)?;
    }
    Ok(output)
}

fn normalize_mount_scope(value: &str) -> String {
    value
        .replace(['/', '\\'], ".")
        .trim_matches('.')
        .trim_end_matches(".py")
        .to_owned()
}

#[allow(clippy::too_many_arguments)]
fn expand_one(
    index: usize,
    routes: &[RawRouteFact],
    by_scope: &HashMap<String, Vec<usize>>,
    limits: FrameworkLimits,
    depth: usize,
    visiting: &mut HashSet<usize>,
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
    if !visiting.insert(index) {
        return Ok(());
    }
    let route = &routes[index];
    let Some(target) = include_target(route) else {
        output.push(route.clone());
        visiting.remove(&index);
        return Ok(());
    };
    let candidates = include_scope_candidates(&target);
    let children = candidates
        .iter()
        .find_map(|candidate| by_scope.get(candidate))
        .cloned()
        .unwrap_or_default();
    if children.is_empty() {
        output.push(route.clone());
        visiting.remove(&index);
        return Ok(());
    }
    for child_index in children {
        if visiting.contains(&child_index) {
            continue;
        }
        let child = &routes[child_index];
        if include_target(child).is_some() {
            let before = output.len();
            expand_one(
                child_index,
                routes,
                by_scope,
                limits,
                depth + 1,
                visiting,
                output,
            )?;
            for expanded in &mut output[before..] {
                compose_parent(route, expanded);
            }
        } else {
            let mut expanded = child.clone();
            compose_parent(route, &mut expanded);
            output.push(expanded);
        }
    }
    visiting.remove(&index);
    Ok(())
}

fn compose_parent(parent: &RawRouteFact, child: &mut RawRouteFact) {
    child.normalized_path = compose_paths(&parent.normalized_path, &child.normalized_path);
    child.raw_path = format!(
        "{}{}",
        parent.raw_path.trim_end_matches('/'),
        child.raw_path.trim_start_matches('/')
    );
    if let Ok(anchor) = serde_json::to_value(&parent.anchor) {
        child.detail.insert("include_anchor".into(), anchor);
    }
    child.detail.insert(
        "include_scope".into(),
        Value::String(parent.declaring_scope.clone()),
    );
}

fn include_target(route: &RawRouteFact) -> Option<String> {
    route
        .detail
        .get("include")
        .and_then(Value::as_str)
        .map(normalize_module)
        .filter(|value| !value.is_empty())
}

fn scope_keys(scope: &str) -> Vec<String> {
    let normalized = normalize_module(scope);
    let mut keys = vec![normalized.clone()];
    if let Some(stripped) = normalized.strip_suffix(".urls") {
        keys.push(stripped.to_owned());
    }
    keys
}

fn include_scope_candidates(target: &str) -> Vec<String> {
    let target = normalize_module(target);
    if target.ends_with(".urls") {
        vec![target]
    } else {
        vec![target.clone(), format!("{target}.urls")]
    }
}

fn normalize_module(value: &str) -> String {
    value
        .trim()
        .trim_matches(['"', '\''])
        .trim_end_matches(".urlpatterns")
        .replace(['/', '\\'], ".")
        .trim_matches('.')
        .to_owned()
}

fn compose_paths(parent: &str, child: &str) -> String {
    let parent = parent.trim_end_matches('/');
    let child = child.trim_start_matches('/');
    if parent.is_empty() {
        format!("/{child}")
    } else if child.is_empty() {
        parent.to_owned()
    } else {
        format!("{parent}/{child}")
    }
}
