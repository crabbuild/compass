//! File-route parentage from framework naming rules, never directory ordering.

use std::collections::BTreeMap;

use compass_languages::RawRouteFact;

use super::{has_filesystem_route_convention, route_hierarchy_scope};

pub(super) struct FileRouteHierarchy {
    by_key: BTreeMap<(String, String, String), Vec<usize>>,
}

impl FileRouteHierarchy {
    pub(super) fn new(routes: &[(String, RawRouteFact)]) -> Self {
        let mut by_key = BTreeMap::<_, Vec<usize>>::new();
        for (index, (_, route)) in routes.iter().enumerate() {
            let Some(path) = RoutePath::for_route(route) else {
                continue;
            };
            if path.can_parent && matches!(route.operation.as_str(), "PAGE" | "ROOT") {
                by_key
                    .entry((route.framework.clone(), path.scope, path.parts.join("/")))
                    .or_default()
                    .push(index);
            }
        }
        for candidates in by_key.values_mut() {
            candidates.sort_by(|left, right| routes[*left].0.cmp(&routes[*right].0));
            candidates.dedup_by(|left, right| routes[*left].0 == routes[*right].0);
        }
        Self { by_key }
    }

    /// Return every candidate at the nearest semantic parent key. An ambiguous
    /// key must not fall back to a more distant parent or an arbitrary file.
    pub(super) fn candidates(&self, child: &RawRouteFact) -> Option<&[usize]> {
        let path = RoutePath::for_route(child)?;
        let first = path.first_parent_len?;
        for length in (path.minimum_parent_len..=first).rev() {
            let key = (
                child.framework.clone(),
                path.scope.clone(),
                path.parts[..length].join("/"),
            );
            if let Some(candidates) = self.by_key.get(&key) {
                return Some(candidates);
            }
        }
        None
    }
}

struct RoutePath {
    scope: String,
    parts: Vec<String>,
    can_parent: bool,
    first_parent_len: Option<usize>,
    minimum_parent_len: usize,
}

impl RoutePath {
    fn for_route(route: &RawRouteFact) -> Option<Self> {
        if !has_filesystem_route_convention(route) {
            return None;
        }
        let scope = route_hierarchy_scope(route);
        let portable = route.anchor.source_file.replace('\\', "/");
        let relative = portable
            .trim_matches('/')
            .strip_prefix(&format!("{scope}/"))?;
        let (stem, extension) = relative.rsplit_once('.')?;
        if stem
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
        {
            return None;
        }
        let mut parts = stem.split('/').map(str::to_owned).collect::<Vec<_>>();
        let file = parts.last()?.as_str();
        let (can_parent, first_parent_len, minimum_parent_len) = match route.framework.as_str() {
            "next" => {
                // Pages Router and HTTP route handlers have no automatic
                // layout parent inferred from their URL or source directory.
                if route.rule.as_deref() != Some("next-app-router-convention")
                    || !matches!(
                        extension,
                        "ts" | "tsx" | "js" | "jsx" | "mts" | "cts" | "mjs" | "cjs"
                    )
                    || !matches!(
                        file,
                        "layout"
                            | "page"
                            | "template"
                            | "error"
                            | "loading"
                            | "not-found"
                            | "default"
                    )
                {
                    return None;
                }
                let layout = file == "layout";
                parts.pop();
                (
                    layout,
                    if layout {
                        parts.len().checked_sub(1)
                    } else {
                        Some(parts.len())
                    },
                    0,
                )
            }
            "sveltekit" => {
                if extension != "svelte" || !matches!(file, "+layout" | "+page" | "+error") {
                    return None;
                }
                let layout = file == "+layout";
                parts.pop();
                (
                    layout,
                    if layout {
                        parts.len().checked_sub(1)
                    } else {
                        Some(parts.len())
                    },
                    0,
                )
            }
            "nuxt" => {
                if extension != "vue" || route.rule.as_deref() != Some("nuxt-file-route-convention")
                {
                    return None;
                }
                // parent.vue owns parent/child.vue; index.vue is a leaf.
                // Named-view and route-group configuration is not represented
                // in these facts, so it cannot establish a parent here.
                if parts
                    .iter()
                    .any(|part| part.contains('@') || part.starts_with('('))
                {
                    return None;
                }
                (file != "index", parts.len().checked_sub(1), 1)
            }
            "react-router" | "remix" | "tanstack-router" => {
                if !matches!(
                    extension,
                    "ts" | "tsx" | "js" | "jsx" | "mts" | "cts" | "mjs" | "cjs"
                ) {
                    return None;
                }
                let tanstack = route.framework == "tanstack-router";
                if !tanstack && parts.len() > 1 {
                    // Flat-route folders expose route.tsx; colocated helper
                    // modules do not define nested routes by directory alone.
                    if parts.len() != 2 || file != "route" {
                        return None;
                    }
                    parts.pop();
                }
                parts = flat_segments(&parts.join("/"), tanstack)?;
                if tanstack && parts.last().is_some_and(|part| part == "route") {
                    parts.pop();
                }
                if tanstack && parts == ["__root"] {
                    parts.clear();
                    (true, None, 0)
                } else {
                    let last = parts.last()?;
                    let can_parent =
                        last != if tanstack { "index" } else { "_index" } && !last.ends_with('_');
                    (can_parent, parts.len().checked_sub(1), 0)
                }
            }
            // Astro layouts are explicit component composition. File routes
            // and API endpoints alone prove no containing route.
            _ => return None,
        };
        Some(Self {
            scope,
            parts,
            can_parent,
            first_parent_len,
            minimum_parent_len,
        })
    }
}

/// Preserve escaped dots as literal filename text. Index, pathless, and
/// non-nesting markers remain in keys, preventing URL normalization from
/// merging distinct route ownership. Every iteration consumes one byte.
fn flat_segments(stem: &str, tanstack: bool) -> Option<Vec<String>> {
    let mut segments = Vec::new();
    let mut start = 0;
    let mut escaped = false;
    for (index, byte) in stem.bytes().enumerate() {
        match byte {
            b'[' if !escaped => escaped = true,
            b']' if escaped => escaped = false,
            b'[' | b']' => return None,
            b'.' | b'/' if !escaped => {
                segments.push(stem.get(start..index)?.to_owned());
                start = index + 1;
            }
            _ => {}
        }
    }
    if escaped {
        return None;
    }
    segments.push(stem.get(start..)?.to_owned());
    if segments.iter().any(|segment| segment.is_empty()) {
        return None;
    }
    if tanstack {
        if segments.iter().any(|segment| segment.starts_with('-'))
            || segments.last().is_some_and(|segment| segment == "lazy")
        {
            return None;
        }
        segments.retain(|segment| !(segment.starts_with('(') && segment.ends_with(')')));
    }
    Some(segments)
}
