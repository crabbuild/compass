use std::collections::{BTreeMap, BTreeSet};

use ahash::{AHashMap as HashMap, AHashSet as HashSet};

use compass_languages::{
    Extraction, FrameworkLimitError, FrameworkLimits, RawDomainFact, RawFrameworkFact, RawRouteFact,
};
use serde_json::Value;

use super::routes::FrameworkResolutionError;

pub(super) fn expand(_extraction: &mut Extraction) -> Result<(), FrameworkResolutionError> {
    Ok(())
}

pub(super) fn expand_django_routes(
    _facts: &[RawFrameworkFact],
    routes: Vec<RawRouteFact>,
    limits: FrameworkLimits,
) -> Result<Vec<RawRouteFact>, FrameworkResolutionError> {
    expand_django_includes(routes, limits)
}

pub(super) fn expand_fastapi_routes(
    facts: &[RawFrameworkFact],
    routes: Vec<RawRouteFact>,
    limits: FrameworkLimits,
) -> Result<Vec<RawRouteFact>, FrameworkResolutionError> {
    expand_router_mounts(facts, routes, limits)
}

pub(super) fn expand_flask_routes(
    facts: &[RawFrameworkFact],
    routes: Vec<RawRouteFact>,
    limits: FrameworkLimits,
) -> Result<Vec<RawRouteFact>, FrameworkResolutionError> {
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
    let mut mounts = facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Domain(domain) if domain.kind == "router_mount" => Some(domain),
            _ => None,
        })
        .collect::<Vec<_>>();
    if mounts.is_empty() {
        return Ok(routes);
    }
    mounts.sort_by(|left, right| mount_sort_key(left).cmp(&mount_sort_key(right)));
    let mut incoming = BTreeMap::<String, Vec<&RawDomainFact>>::new();
    let mut receiver_ids = BTreeMap::<String, BTreeSet<String>>::new();
    for mount in &mounts {
        let Some(target) = mount_string(mount, "target_receiver_qualified_name") else {
            continue;
        };
        let Some(parent) = mount_string(mount, "parent_receiver_qualified_name") else {
            continue;
        };
        incoming.entry(target.to_owned()).or_default().push(mount);
        if let Some(id) = mount_string(mount, "target_receiver_id") {
            receiver_ids
                .entry(target.to_owned())
                .or_default()
                .insert(id.to_owned());
        }
        if let Some(id) = mount_string(mount, "parent_receiver_id") {
            receiver_ids
                .entry(parent.to_owned())
                .or_default()
                .insert(id.to_owned());
        }
    }
    for route in &routes {
        if let (Some(receiver), Some(id)) = (
            route
                .detail
                .get("receiver_qualified_name")
                .and_then(Value::as_str),
            route.detail.get("receiver_id").and_then(Value::as_str),
        ) {
            receiver_ids
                .entry(receiver.to_owned())
                .or_default()
                .insert(id.to_owned());
        }
        if let (Some(receiver), Some(id)) = (
            route
                .detail
                .get("mounted_receiver_qualified_name")
                .and_then(Value::as_str),
            route
                .detail
                .get("mounted_receiver_id")
                .and_then(Value::as_str),
        ) {
            receiver_ids
                .entry(receiver.to_owned())
                .or_default()
                .insert(id.to_owned());
        }
    }
    let mut output = Vec::new();
    for route in routes {
        let Some(receiver) = route
            .detail
            .get("mounted_receiver_qualified_name")
            .or_else(|| route.detail.get("receiver_qualified_name"))
            .and_then(Value::as_str)
        else {
            output.push(route);
            continue;
        };
        if receiver_ids.get(receiver).is_some_and(|ids| ids.len() > 1) {
            // A qualified receiver that names multiple declarations cannot
            // safely participate in a mount edge. Retain the unmounted route
            // rather than selecting a declaration by merge/source order.
            output.push(route);
            continue;
        }
        let mut visiting = BTreeSet::new();
        let chains = mount_chains(
            receiver,
            &route.framework,
            &incoming,
            limits,
            0,
            &mut visiting,
        )?;
        if chains.len() == 1 && chains[0].is_empty() {
            output.push(route);
            continue;
        }
        for chain in chains {
            let prefix = chain.iter().fold(String::new(), |prefix, mount| {
                compose_paths(
                    &prefix,
                    mount_string(mount, "mount_prefix").unwrap_or_default(),
                )
            });
            let mut expanded = route.clone();
            expanded.normalized_path = compose_paths(&prefix, &route.normalized_path);
            expanded.raw_path = compose_paths(&prefix, &route.raw_path);
            expanded
                .detail
                .insert("mount_prefix".into(), Value::String(prefix));
            let anchors = chain
                .iter()
                .map(|mount| serde_json::to_value(&mount.anchor).unwrap_or(Value::Null))
                .collect::<Vec<_>>();
            if let Some(anchor) = anchors.last() {
                expanded
                    .detail
                    .insert("mount_anchor".into(), anchor.clone());
            }
            expanded
                .detail
                .insert("mount_anchors".into(), Value::Array(anchors));
            expanded.rule = Some("python-receiver-mount".to_owned());
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

fn mount_chains<'a>(
    receiver: &str,
    framework: &str,
    incoming: &BTreeMap<String, Vec<&'a RawDomainFact>>,
    limits: FrameworkLimits,
    depth: usize,
    visiting: &mut BTreeSet<String>,
) -> Result<Vec<Vec<&'a RawDomainFact>>, FrameworkResolutionError> {
    if !visiting.insert(receiver.to_owned()) {
        return Ok(Vec::new());
    }
    let applicable = incoming
        .get(receiver)
        .into_iter()
        .flatten()
        .filter(|mount| mount.framework == framework)
        .copied()
        .collect::<Vec<_>>();
    if applicable.is_empty() {
        visiting.remove(receiver);
        return Ok(vec![Vec::new()]);
    }
    if depth >= limits.max_include_depth {
        visiting.remove(receiver);
        return Err(FrameworkLimitError {
            limit: "max_include_depth",
            maximum: limits.max_include_depth,
            observed: depth.saturating_add(1),
        }
        .into());
    }
    let mut chains = Vec::new();
    for mount in applicable {
        let Some(parent) = mount_string(mount, "parent_receiver_qualified_name") else {
            continue;
        };
        let outer = mount_chains(
            parent,
            framework,
            incoming,
            limits,
            depth.saturating_add(1),
            visiting,
        )?;
        for mut chain in outer {
            chain.push(mount);
            chains.push(chain);
        }
    }
    visiting.remove(receiver);
    Ok(chains)
}

fn mount_string<'a>(mount: &'a RawDomainFact, key: &str) -> Option<&'a str> {
    mount.detail.get(key).and_then(Value::as_str)
}

fn mount_sort_key(mount: &RawDomainFact) -> (&str, &str, &str, &str, u64, u64) {
    (
        mount_string(mount, "target_receiver_qualified_name").unwrap_or_default(),
        mount_string(mount, "parent_receiver_qualified_name").unwrap_or_default(),
        mount_string(mount, "mount_prefix").unwrap_or_default(),
        mount.anchor.source_file.as_str(),
        mount.anchor.start_byte,
        mount.anchor.end_byte,
    )
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
