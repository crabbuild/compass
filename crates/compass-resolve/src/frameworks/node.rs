use std::collections::{BTreeMap, BTreeSet, VecDeque};

use compass_languages::{
    FrameworkLimitError, FrameworkLimits, RawDomainFact, RawFrameworkAnchor, RawFrameworkFact,
    RawRouteFact,
};
use serde_json::Value;

use super::routes::FrameworkResolutionError;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct RouterKey {
    scope: String,
    receiver: String,
}

#[derive(Clone)]
struct RouterMount {
    parent: RouterKey,
    prefix: String,
    anchor: RawFrameworkAnchor,
}

pub(super) fn expand_routes(
    facts: &[RawFrameworkFact],
    routes: Vec<RawRouteFact>,
    limits: FrameworkLimits,
) -> Result<Vec<RawRouteFact>, FrameworkResolutionError> {
    let mounts = facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Domain(domain)
                if domain.kind == "router_mount"
                    && domain
                        .detail
                        .get("target_module")
                        .and_then(Value::as_str)
                        .is_some_and(|module| module.starts_with('.')) =>
            {
                Some(domain)
            }
            RawFrameworkFact::Route(_)
            | RawFrameworkFact::Domain(_)
            | RawFrameworkFact::Annotation(_) => None,
        })
        .collect::<Vec<_>>();
    if mounts.is_empty() {
        return Ok(routes);
    }

    let mut owners = BTreeSet::new();
    for route in &routes {
        if let Some(receiver) = route.detail.get("receiver").and_then(Value::as_str) {
            owners.insert(RouterKey {
                scope: normalize_scope(&route.declaring_scope),
                receiver: receiver.to_owned(),
            });
        }
    }
    for mount in &mounts {
        if let Some(parent) = mount.detail.get("parent_receiver").and_then(Value::as_str) {
            owners.insert(RouterKey {
                scope: normalize_scope(&mount.declaring_scope),
                receiver: parent.to_owned(),
            });
        }
    }

    let mut incoming = BTreeMap::<RouterKey, Vec<RouterMount>>::new();
    for mount in mounts {
        let Some((parent, target)) = resolve_mount(mount, &owners) else {
            continue;
        };
        incoming.entry(target).or_default().push(RouterMount {
            parent,
            prefix: mount
                .detail
                .get("mount_prefix")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            anchor: mount.anchor.clone(),
        });
    }
    for values in incoming.values_mut() {
        values.sort_by(|left, right| {
            (
                &left.parent,
                &left.prefix,
                &left.anchor.source_file,
                left.anchor.start_byte,
                left.anchor.end_byte,
            )
                .cmp(&(
                    &right.parent,
                    &right.prefix,
                    &right.anchor.source_file,
                    right.anchor.start_byte,
                    right.anchor.end_byte,
                ))
        });
        values.dedup_by(|left, right| {
            left.parent == right.parent
                && left.prefix == right.prefix
                && left.anchor == right.anchor
        });
    }

    let mut output = Vec::new();
    for route in routes {
        let Some(receiver) = route.detail.get("receiver").and_then(Value::as_str) else {
            output.push(route);
            continue;
        };
        let start = RouterKey {
            scope: normalize_scope(&route.declaring_scope),
            receiver: receiver.to_owned(),
        };
        if !incoming.contains_key(&start) {
            output.push(route);
            continue;
        }
        let local_prefix = route
            .detail
            .get("mount_prefix")
            .and_then(Value::as_str)
            .filter(|prefix| !prefix.is_empty())
            .map(str::to_owned);
        let initial_raw_path = local_prefix.as_deref().map_or_else(
            || route.raw_path.clone(),
            |prefix| compose_paths(prefix, &route.raw_path),
        );
        let initial_prefixes = local_prefix.into_iter().collect::<Vec<_>>();
        let mut expanded = Vec::new();
        let mut queue = VecDeque::from([(
            start.clone(),
            route.normalized_path.clone(),
            initial_raw_path,
            initial_prefixes,
            Vec::<RawFrameworkAnchor>::new(),
            BTreeSet::from([start]),
        )]);
        while let Some((owner, path, raw_path, prefixes, anchors, visited)) = queue.pop_front() {
            let Some(parent_mounts) = incoming.get(&owner) else {
                let mut composed = route.clone();
                composed.normalized_path = path;
                composed.raw_path = raw_path;
                composed.detail.insert(
                    "mount_prefix".into(),
                    Value::String(
                        prefixes
                            .iter()
                            .fold(String::new(), |path, prefix| compose_paths(prefix, &path)),
                    ),
                );
                composed
                    .detail
                    .insert("mount_anchors".into(), anchors_value(&anchors));
                expanded.push(composed);
                continue;
            };
            if visited.len() > limits.max_include_depth {
                return Err(FrameworkLimitError {
                    limit: "max_include_depth",
                    maximum: limits.max_include_depth,
                    observed: visited.len(),
                }
                .into());
            }
            for mount in parent_mounts {
                if visited.contains(&mount.parent) {
                    continue;
                }
                let mut next_prefixes = prefixes.clone();
                next_prefixes.push(mount.prefix.clone());
                let mut next_anchors = anchors.clone();
                next_anchors.push(mount.anchor.clone());
                let mut next_visited = visited.clone();
                next_visited.insert(mount.parent.clone());
                queue.push_back((
                    mount.parent.clone(),
                    compose_paths(&mount.prefix, &path),
                    compose_paths(&mount.prefix, &raw_path),
                    next_prefixes,
                    next_anchors,
                    next_visited,
                ));
            }
        }
        if expanded.is_empty() {
            output.push(route);
        } else {
            output.extend(expanded);
        }
    }
    let mut per_file = BTreeMap::new();
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

fn resolve_mount(
    mount: &RawDomainFact,
    owners: &BTreeSet<RouterKey>,
) -> Option<(RouterKey, RouterKey)> {
    let parent_receiver = mount
        .detail
        .get("parent_receiver")
        .and_then(Value::as_str)?;
    let target_receiver = mount
        .detail
        .get("target_receiver")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let target_module = mount.detail.get("target_module").and_then(Value::as_str)?;
    let parent_scope = normalize_scope(&mount.declaring_scope);
    let target_scope = resolve_relative_scope(&parent_scope, target_module)?;
    let candidates = owners
        .iter()
        .filter(|owner| {
            owner.scope == target_scope
                || owner.scope == format!("{target_scope}.index")
                || owner.scope.ends_with(&format!(".{target_scope}"))
        })
        .cloned()
        .collect::<Vec<_>>();
    let target = candidates
        .iter()
        .find(|owner| owner.receiver == target_receiver)
        .cloned()
        .or_else(|| (candidates.len() == 1).then(|| candidates[0].clone()))?;
    Some((
        RouterKey {
            scope: parent_scope,
            receiver: parent_receiver.to_owned(),
        },
        target,
    ))
}

fn resolve_relative_scope(parent_scope: &str, module: &str) -> Option<String> {
    if !module.starts_with('.') {
        return None;
    }
    let mut segments = parent_scope
        .split('.')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    segments.pop();
    let normalized_module = module.replace('\\', "/");
    for segment in normalized_module.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            value => segments.push(
                value
                    .trim_end_matches(".tsx")
                    .trim_end_matches(".ts")
                    .trim_end_matches(".jsx")
                    .trim_end_matches(".js"),
            ),
        }
    }
    (!segments.is_empty()).then(|| segments.join("."))
}

fn normalize_scope(value: &str) -> String {
    value
        .replace(['/', '\\'], ".")
        .trim_matches('.')
        .trim_end_matches(".tsx")
        .trim_end_matches(".ts")
        .trim_end_matches(".jsx")
        .trim_end_matches(".js")
        .to_owned()
}

fn compose_paths(prefix: &str, path: &str) -> String {
    let prefix = prefix.trim_matches('/');
    let path = path.trim_matches('/');
    match (prefix.is_empty(), path.is_empty()) {
        (true, true) => "/".to_owned(),
        (true, false) => format!("/{path}"),
        (false, true) => format!("/{prefix}"),
        (false, false) => format!("/{prefix}/{path}"),
    }
}

fn anchors_value(anchors: &[RawFrameworkAnchor]) -> Value {
    Value::Array(
        anchors
            .iter()
            .map(|anchor| {
                serde_json::json!({
                    "sourceFile": anchor.source_file,
                    "startByte": anchor.start_byte,
                    "endByte": anchor.end_byte,
                    "startLine": anchor.start_line,
                    "startColumn": anchor.start_column,
                    "endLine": anchor.end_line,
                    "endColumn": anchor.end_column,
                })
            })
            .collect(),
    )
}
