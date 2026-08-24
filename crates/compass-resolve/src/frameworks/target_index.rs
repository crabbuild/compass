use std::path::Path;

use ahash::{AHashMap as HashMap, AHashSet as HashSet};
use compass_languages::{Extraction, RawNodeRecord};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum TargetFamily {
    Route,
    Callable,
    Type,
    DatabaseTable,
}

pub(super) struct IndexedTarget<'a> {
    pub node: &'a RawNodeRecord,
}

pub(super) struct FrameworkTargetIndex<'a> {
    pub targets: Vec<IndexedTarget<'a>>,
    root: Option<&'a Path>,
    by_id: HashMap<(TargetFamily, &'a str), Vec<usize>>,
    by_qualified: HashMap<(TargetFamily, String), Vec<usize>>,
    by_terminal: HashMap<(TargetFamily, String), Vec<usize>>,
    by_source_terminal: HashMap<(TargetFamily, String, String), Vec<usize>>,
    by_owner_terminal: HashMap<(TargetFamily, String, String), Vec<usize>>,
    by_module_terminal: HashMap<(TargetFamily, String, String), Vec<usize>>,
}

impl<'a> FrameworkTargetIndex<'a> {
    pub fn new(extraction: &'a Extraction) -> Self {
        Self::new_with_root(extraction, None)
    }

    pub fn new_with_root(extraction: &'a Extraction, root: Option<&'a Path>) -> Self {
        let raw_by_id = extraction
            .nodes
            .iter()
            .map(|node| (node.id.as_str(), node))
            .collect::<HashMap<_, _>>();
        let target_ids = extraction
            .nodes
            .iter()
            .filter(|node| !target_families(node).is_empty())
            .map(|node| node.id.as_str())
            .collect::<HashSet<_>>();
        let mut owners = HashMap::<&str, Vec<String>>::new();
        for edge in &extraction.edges {
            if !matches!(
                edge.attributes.get("relation").and_then(Value::as_str),
                Some("contains" | "method" | "defines")
            ) || !target_ids.contains(edge.target.as_str())
            {
                continue;
            }
            if let Some(parent) = raw_by_id.get(edge.source.as_str()) {
                owners.entry(edge.target.as_str()).or_default().extend(
                    [
                        parent.string("qualified_name"),
                        parent.string("name"),
                        parent.label().to_owned(),
                    ]
                    .into_iter()
                    .map(|name| normalize_reference(&name))
                    .filter(|name| !name.is_empty()),
                );
            }
        }
        for values in owners.values_mut() {
            values.sort();
            values.dedup();
        }

        let mut index = Self {
            targets: Vec::with_capacity(extraction.nodes.len()),
            root,
            by_id: HashMap::new(),
            by_qualified: HashMap::new(),
            by_terminal: HashMap::new(),
            by_source_terminal: HashMap::new(),
            by_owner_terminal: HashMap::new(),
            by_module_terminal: HashMap::new(),
        };
        for node in &extraction.nodes {
            let families = target_families(node);
            if families.is_empty() {
                continue;
            }
            let qualified = normalize_reference(&node.string("qualified_name"));
            let signature_qualified = node
                .attributes
                .get("signature")
                .and_then(Value::as_str)
                .and_then(|signature| signature.find('(').map(|start| &signature[start..]))
                .filter(|_| !qualified.is_empty())
                .map(|parameters| format!("{qualified}{parameters}"));
            let raw_source = node
                .attributes
                .get("source_file")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let source = source_key(raw_source, root);
            let mut normalized = [
                node.string("qualified_name"),
                node.string("name"),
                node.label().to_owned(),
                node.attributes
                    .get("export_name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            ]
            .into_iter()
            .map(|name| normalize_reference(&name))
            .filter(|name| !name.is_empty())
            .collect::<Vec<_>>();
            normalized.sort();
            normalized.dedup();
            let target_owners = owners.remove(node.id.as_str()).unwrap_or_default();
            let position = index.targets.len();
            for &family in families {
                index
                    .by_id
                    .entry((family, node.id.as_str()))
                    .or_default()
                    .push(position);
                if !qualified.is_empty() {
                    index
                        .by_qualified
                        .entry((family, qualified.clone()))
                        .or_default()
                        .push(position);
                }
                if let Some(signature_qualified) = signature_qualified.as_ref() {
                    index
                        .by_qualified
                        .entry((family, signature_qualified.clone()))
                        .or_default()
                        .push(position);
                }
                for name in &normalized {
                    let terminal = terminal_name(name).to_owned();
                    index
                        .by_terminal
                        .entry((family, terminal.clone()))
                        .or_default()
                        .push(position);
                    if !source.is_empty() {
                        index
                            .by_source_terminal
                            .entry((family, source.clone(), terminal.clone()))
                            .or_default()
                            .push(position);
                        index
                            .by_module_terminal
                            .entry((family, module_key(&source), terminal.clone()))
                            .or_default()
                            .push(position);
                    }
                    for owner in &target_owners {
                        index
                            .by_owner_terminal
                            .entry((family, terminal_name(owner).to_owned(), terminal.clone()))
                            .or_default()
                            .push(position);
                    }
                }
            }
            index.targets.push(IndexedTarget { node });
        }
        for positions in index
            .by_id
            .values_mut()
            .chain(index.by_qualified.values_mut())
            .chain(index.by_terminal.values_mut())
            .chain(index.by_source_terminal.values_mut())
            .chain(index.by_owner_terminal.values_mut())
            .chain(index.by_module_terminal.values_mut())
        {
            positions.sort_unstable();
            positions.dedup();
        }
        index
    }

    pub fn by_id(
        &self,
        value: &str,
        families: &[TargetFamily],
        limit: usize,
    ) -> (Vec<usize>, bool) {
        bounded_union(
            families
                .iter()
                .filter_map(|family| self.by_id.get(&(*family, value)).map(Vec::as_slice)),
            limit,
        )
    }

    pub fn by_names(
        &self,
        values: &[String],
        families: &[TargetFamily],
        limit: usize,
    ) -> (Vec<usize>, bool) {
        bounded_union(
            values.iter().flat_map(|value| {
                families.iter().filter_map(|family| {
                    self.by_qualified
                        .get(&(*family, value.clone()))
                        .map(Vec::as_slice)
                })
            }),
            limit,
        )
    }

    pub fn by_terminal(
        &self,
        value: &str,
        families: &[TargetFamily],
        limit: usize,
    ) -> (Vec<usize>, bool) {
        bounded_union(
            families.iter().filter_map(|family| {
                self.by_terminal
                    .get(&(*family, value.to_owned()))
                    .map(Vec::as_slice)
            }),
            limit,
        )
    }

    pub fn by_source_terminal(
        &self,
        source: &str,
        terminal: &str,
        families: &[TargetFamily],
        limit: usize,
    ) -> (Vec<usize>, bool) {
        let source = source_key(source, self.root);
        let direct = bounded_union(
            families.iter().filter_map(|family| {
                self.by_source_terminal
                    .get(&(*family, source.clone(), terminal.to_owned()))
                    .map(Vec::as_slice)
            }),
            limit,
        );
        if !direct.0.is_empty() || direct.1 {
            return direct;
        }
        // Universal TypeScript/JavaScript evidence deliberately carries a
        // portable source suffix when a caller has no project root. Framework
        // facts, however, may still retain the absolute path used for route
        // convention detection. Treat an exact path suffix as the same source
        // only for this source-scoped lookup; if several suffixes match, the
        // normal candidate-state logic reports ambiguity instead of guessing.
        bounded_union(
            families.iter().flat_map(|family| {
                self.by_source_terminal.iter().filter_map(
                    |((indexed_family, indexed_source, indexed_terminal), values)| {
                        (*indexed_family == *family
                            && indexed_terminal == terminal
                            && source_suffix_matches(indexed_source, &source))
                        .then_some(values.as_slice())
                    },
                )
            }),
            limit,
        )
    }

    pub fn by_owner_terminal(
        &self,
        owner: &str,
        terminal: &str,
        families: &[TargetFamily],
        limit: usize,
    ) -> (Vec<usize>, bool) {
        let owner = terminal_name(owner).to_owned();
        bounded_union(
            families.iter().filter_map(|family| {
                self.by_owner_terminal
                    .get(&(*family, owner.clone(), terminal.to_owned()))
                    .map(Vec::as_slice)
            }),
            limit,
        )
    }

    pub fn by_module_terminal(
        &self,
        declaring_source: &str,
        module: &str,
        terminal: &str,
        families: &[TargetFamily],
        limit: usize,
    ) -> (Vec<usize>, bool) {
        let declaring_source = source_key(declaring_source, self.root);
        let parent = Path::new(&declaring_source)
            .parent()
            .unwrap_or_else(|| std::path::Path::new(""));
        let module = module_key(&lexical_path(&parent.join(module)));
        bounded_union(
            families.iter().filter_map(|family| {
                self.by_module_terminal
                    .get(&(*family, module.clone(), terminal.to_owned()))
                    .map(Vec::as_slice)
            }),
            limit,
        )
    }
}

fn source_suffix_matches(left: &str, right: &str) -> bool {
    let left = left.trim_start_matches("./");
    let right = right.trim_start_matches("./");
    left == right
        || left
            .strip_prefix('/')
            .is_some_and(|left| left == right || left.ends_with(&format!("/{right}")))
        || right
            .strip_prefix('/')
            .is_some_and(|right| right == left || right.ends_with(&format!("/{left}")))
        || left.ends_with(&format!("/{right}"))
        || right.ends_with(&format!("/{left}"))
}

pub(super) fn source_key(source: &str, root: Option<&Path>) -> String {
    let path = Path::new(source);
    if path.is_absolute()
        && let Some(root) = root
        && let Ok(relative) = path.strip_prefix(root)
    {
        return relative.to_string_lossy().replace('\\', "/");
    }
    source.replace('\\', "/")
}

fn bounded_union<'a>(
    buckets: impl Iterator<Item = &'a [usize]>,
    limit: usize,
) -> (Vec<usize>, bool) {
    let (retained, truncated, _) = bounded_union_measured(buckets, limit);
    (retained, truncated)
}

fn bounded_union_measured<'a>(
    buckets: impl Iterator<Item = &'a [usize]>,
    limit: usize,
) -> (Vec<usize>, bool, usize) {
    let buckets = buckets.collect::<Vec<_>>();
    let mut positions = vec![0_usize; buckets.len()];
    let mut retained = Vec::with_capacity(limit.min(64).saturating_add(1));
    let mut seen = HashSet::with_capacity(limit.min(64).saturating_add(1));
    let mut examined = 0_usize;
    while retained.len() <= limit {
        examined = examined.saturating_add(buckets.len());
        let next = buckets
            .iter()
            .enumerate()
            .filter_map(|(bucket, values)| {
                values
                    .get(positions[bucket])
                    .copied()
                    .map(|value| (bucket, value))
            })
            .min_by_key(|(_, value)| *value);
        let Some((bucket, value)) = next else {
            break;
        };
        positions[bucket] += 1;
        if seen.insert(value) {
            retained.push(value);
        }
    }
    let truncated = retained.len() > limit;
    if truncated {
        retained.truncate(limit);
    }
    (retained, truncated, examined)
}

fn target_families(node: &RawNodeRecord) -> &'static [TargetFamily] {
    const ROUTE_CALLABLE: &[TargetFamily] = &[TargetFamily::Route, TargetFamily::Callable];
    const ROUTE_TYPE: &[TargetFamily] = &[TargetFamily::Route, TargetFamily::Type];
    const ROUTE: &[TargetFamily] = &[TargetFamily::Route];
    const TYPE: &[TargetFamily] = &[TargetFamily::Type];
    const DATABASE_TABLE: &[TargetFamily] = &[TargetFamily::DatabaseTable];
    let kind = node
        .attributes
        .get("symbol_kind")
        .or_else(|| node.attributes.get("type"))
        .and_then(Value::as_str);
    match kind {
        Some("function" | "method") => ROUTE_CALLABLE,
        Some("class") => ROUTE_TYPE,
        Some("component") => ROUTE,
        Some("struct" | "interface" | "trait" | "protocol" | "enum") => TYPE,
        Some("database_table" | "table") => DATABASE_TABLE,
        _ => &[],
    }
}

pub(super) fn normalize_reference(value: &str) -> String {
    value
        .trim()
        .trim_matches(['"', '\'', '`'])
        .trim_start_matches(['&', '*'])
        .trim_end_matches(".as_view()")
        .trim_end_matches("()")
        .replace(['\\', '#'], ".")
        .replace("::", ".")
}

pub(super) fn terminal_name(value: &str) -> &str {
    value.rsplit(['.', ':', '#']).next().unwrap_or(value)
}

fn module_key(path: &str) -> String {
    let path = path.replace('\\', "/");
    let path = [
        ".tsx", ".ts", ".jsx", ".js", ".mts", ".mjs", ".vue", ".py", ".rb", ".php", ".java", ".cs",
        ".go", ".rs", ".swift",
    ]
    .into_iter()
    .find_map(|extension| path.strip_suffix(extension))
    .unwrap_or(&path);
    path.strip_suffix("/index").unwrap_or(path).to_owned()
}

fn lexical_path(path: &std::path::Path) -> String {
    let mut output = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                output.pop();
            }
            std::path::Component::Normal(value) => output.push(value),
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                output.push(component.as_os_str());
            }
        }
    }
    output.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use compass_languages::{Extraction, RawNodeRecord};
    use serde_json::{Map, Value};

    use super::{FrameworkTargetIndex, TargetFamily, bounded_union_measured};

    #[test]
    fn ambiguous_terminal_lookup_is_bounded_by_candidate_budget() {
        const TARGETS: usize = 100_000;
        const LIMIT: usize = 20;

        let mut extraction = Extraction::default();
        extraction.nodes.reserve(TARGETS);
        for index in 0..TARGETS {
            extraction.nodes.push(RawNodeRecord {
                id: format!("node:{index:06}"),
                attributes: Map::from_iter([
                    ("label".to_owned(), Value::String("handler".to_owned())),
                    ("name".to_owned(), Value::String("handler".to_owned())),
                    (
                        "qualified_name".to_owned(),
                        Value::String(format!("module_{index:06}.handler")),
                    ),
                    (
                        "symbol_kind".to_owned(),
                        Value::String("function".to_owned()),
                    ),
                    (
                        "source_file".to_owned(),
                        Value::String(format!("src/module_{index:06}.rs")),
                    ),
                ]),
            });
        }

        let index = FrameworkTargetIndex::new(&extraction);
        let key = (TargetFamily::Route, "handler".to_owned());
        assert!(index.by_terminal.contains_key(&key));
        let bucket = &index.by_terminal[&key];
        let (retained, truncated, examined) =
            bounded_union_measured(std::iter::once(bucket.as_slice()), LIMIT);

        assert_eq!(retained.len(), LIMIT);
        assert!(truncated);
        assert_eq!(examined, LIMIT + 1);
        println!(
            "{{\"targets\":{TARGETS},\"retained\":{},\"examined\":{examined}}}",
            retained.len()
        );
    }

    #[test]
    fn rooted_index_matches_portable_framework_source() {
        let root = std::env::temp_dir().join("compass-framework-target-index-repository");
        let source = root.join("routes/python/django/qualification/urls.py");
        let mut extraction = Extraction::default();
        extraction.nodes.push(RawNodeRecord {
            id: "handler".to_owned(),
            attributes: Map::from_iter([
                (
                    "label".to_owned(),
                    Value::String("qualification_health()".to_owned()),
                ),
                (
                    "qualified_name".to_owned(),
                    Value::String("qualification_health()".to_owned()),
                ),
                (
                    "symbol_kind".to_owned(),
                    Value::String("function".to_owned()),
                ),
                (
                    "source_file".to_owned(),
                    Value::String(source.to_string_lossy().into_owned()),
                ),
            ]),
        });

        let index = FrameworkTargetIndex::new_with_root(&extraction, Some(&root));
        let (positions, truncated) = index.by_source_terminal(
            "routes/python/django/qualification/urls.py",
            "qualification_health",
            &[TargetFamily::Route],
            8,
        );

        assert_eq!(positions, vec![0]);
        assert!(!truncated);
    }
}
