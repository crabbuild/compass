use std::collections::{BTreeMap, BTreeSet};

use ahash::{AHashMap as HashMap, AHashSet as HashSet};

use compass_languages::{
    Extraction, FrameworkLimitError, FrameworkLimits, RawDomainFact, RawFrameworkFact,
    RawRouteFact, RawRouteStageFact,
};
use serde_json::{Map, Value};

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

pub(super) fn expand_drf_routes(
    facts: &[RawFrameworkFact],
    routes: Vec<RawRouteFact>,
    limits: FrameworkLimits,
) -> Result<Vec<RawRouteFact>, FrameworkResolutionError> {
    let mut output = routes;
    let mut registrations = facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Domain(domain) if domain.kind == "drf_router_registration" => {
                Some(domain)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut mounts = facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Domain(domain) if domain.kind == "drf_router_mount" => Some(domain),
            _ => None,
        })
        .collect::<Vec<_>>();
    registrations.sort_by_key(|fact| {
        (
            fact.anchor.source_file.as_str(),
            fact.anchor.start_byte,
            fact.anchor.end_byte,
            fact.name.as_str(),
        )
    });
    mounts.sort_by_key(|fact| {
        (
            fact.anchor.source_file.as_str(),
            fact.anchor.start_byte,
            fact.anchor.end_byte,
            fact.name.as_str(),
        )
    });
    let mut identities = BTreeMap::<&str, BTreeSet<&str>>::new();
    for fact in registrations.iter().chain(mounts.iter()) {
        if let (Some(qualified), Some(id)) = (
            fact.detail
                .get("router_receiver_qualified_name")
                .and_then(Value::as_str),
            fact.detail
                .get("router_receiver_id")
                .and_then(Value::as_str),
        ) {
            identities.entry(qualified).or_default().insert(id);
        }
    }
    for registration in registrations {
        let Some(router) = registration
            .detail
            .get("router_receiver_qualified_name")
            .and_then(Value::as_str)
        else {
            continue;
        };
        let Some(router_id) = registration
            .detail
            .get("router_receiver_id")
            .and_then(Value::as_str)
        else {
            continue;
        };
        if identities.get(router).is_none_or(|ids| ids.len() != 1) {
            continue;
        }
        let applicable_mounts = mounts.iter().filter(|mount| {
            mount
                .detail
                .get("router_receiver_qualified_name")
                .and_then(Value::as_str)
                == Some(router)
                && mount
                    .detail
                    .get("router_receiver_id")
                    .and_then(Value::as_str)
                    == Some(router_id)
                && (mount.anchor.source_file != registration.anchor.source_file
                    || registration.anchor.start_byte < mount.anchor.start_byte)
        });
        for mount in applicable_mounts {
            append_drf_registration_routes(registration, mount, &mut output);
        }
    }
    let mut per_file = BTreeMap::<&str, usize>::new();
    for route in &output {
        *per_file
            .entry(route.anchor.source_file.as_str())
            .or_default() += 1;
    }
    for count in per_file.into_values() {
        limits.check_facts(count)?;
    }
    Ok(output)
}

fn append_drf_registration_routes(
    registration: &RawDomainFact,
    mount: &RawDomainFact,
    output: &mut Vec<RawRouteFact>,
) {
    let Some(methods) = registration.detail.get("methods").and_then(Value::as_array) else {
        return;
    };
    let registration_prefix = registration
        .detail
        .get("prefix")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mount_prefix = mount
        .detail
        .get("mount_prefix")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let template = registration
        .detail
        .get("router_template")
        .and_then(Value::as_str)
        .unwrap_or("drf-simple-router-v1");
    for method in methods {
        let Some(method) = method.as_object() else {
            continue;
        };
        let Some(operation) = method.get("operation").and_then(Value::as_str) else {
            continue;
        };
        let Some(handler) = method.get("handler").and_then(Value::as_str) else {
            continue;
        };
        let detail = method
            .get("detail")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let Some(lookup_parameter) = registration
            .detail
            .get("lookup_parameter")
            .and_then(Value::as_str)
        else {
            continue;
        };
        let url_path = method.get("url_path").and_then(Value::as_str);
        let mut path = compose_paths(mount_prefix, registration_prefix);
        if detail {
            path = compose_paths(&path, &format!("{{{lookup_parameter}}}"));
        }
        if let Some(url_path) = url_path {
            path = compose_paths(&path, url_path);
        }
        let mut route_detail = Map::from_iter([
            (
                "pack_id".to_owned(),
                Value::String("django-rest-framework-python".to_owned()),
            ),
            (
                "router_template".to_owned(),
                Value::String(template.to_owned()),
            ),
            (
                "router_receiver_id".to_owned(),
                registration
                    .detail
                    .get("router_receiver_id")
                    .cloned()
                    .unwrap_or(Value::Null),
            ),
            (
                "viewset_reference".to_owned(),
                registration
                    .detail
                    .get("viewset_reference")
                    .cloned()
                    .unwrap_or(Value::Null),
            ),
            (
                "registration_anchor".to_owned(),
                serde_json::to_value(&registration.anchor).unwrap_or(Value::Null),
            ),
            (
                "mount_anchor".to_owned(),
                serde_json::to_value(&mount.anchor).unwrap_or(Value::Null),
            ),
        ]);
        if let Some(namespace) = mount
            .detail
            .get("namespace")
            .filter(|value| !value.is_null())
        {
            route_detail.insert("namespace".to_owned(), namespace.clone());
        }
        output.push(RawRouteFact {
            framework: "django-rest-framework".to_owned(),
            operation: operation.to_owned(),
            raw_path: path.clone(),
            normalized_path: path,
            declaring_scope: registration.declaring_scope.clone(),
            anchor: registration.anchor.clone(),
            handler_reference: handler.to_owned(),
            middleware_references: Vec::new(),
            stages: Vec::new(),
            origin: compass_languages::RawFrameworkOrigin::Convention,
            rule: Some(template.to_owned()),
            detail: route_detail,
        });
    }
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

pub(super) fn expand_starlette_routes(
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
    let included_targets = routes
        .iter()
        .filter_map(|route| {
            include_target(route).map(|target| {
                (
                    include_scope_candidates(&target),
                    route
                        .detail
                        .get("include_collection")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                )
            })
        })
        .collect::<Vec<_>>();

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
            || !included_targets.iter().any(|(scopes, collection)| {
                scope_keys(&route.declaring_scope)
                    .iter()
                    .any(|scope| scopes.contains(scope))
                    && collection.as_deref().is_none_or(|collection| {
                        route
                            .detail
                            .get("django_collection")
                            .and_then(Value::as_str)
                            == Some(collection)
                    })
            })
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
            let inherited_stages = chain
                .iter()
                .flat_map(|mount| {
                    mount_stage_facts(mount, "parent_stages")
                        .into_iter()
                        .chain(mount_stage_facts(mount, "mount_stages"))
                })
                .collect::<Vec<_>>();
            if !inherited_stages.is_empty() {
                expanded.stages = inherited_stages
                    .into_iter()
                    .chain(expanded.stages)
                    .enumerate()
                    .map(|(position, mut stage)| {
                        stage.position = u32::try_from(position).unwrap_or(u32::MAX);
                        stage
                    })
                    .collect();
            }
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

fn mount_stage_facts(mount: &RawDomainFact, key: &str) -> Vec<RawRouteStageFact> {
    mount
        .detail
        .get(key)
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
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
    let mut children = candidates
        .iter()
        .find_map(|candidate| by_scope.get(candidate))
        .cloned()
        .unwrap_or_default();
    if let Some(collection) = route
        .detail
        .get("include_collection")
        .and_then(Value::as_str)
    {
        children.retain(|child| {
            routes[*child]
                .detail
                .get("django_collection")
                .and_then(Value::as_str)
                == Some(collection)
        });
    }
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
