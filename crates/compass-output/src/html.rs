use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;

use compass_files::write_text_atomic;
use compass_graph::Communities;
use compass_model::{EdgeRecord, GraphDocument, NodeRecord};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::OutputError;
use crate::json::python_json_compact;

const DEFAULT_NODE_LIMIT: isize = 5_000;
const COMMUNITY_COLORS: [&str; 10] = [
    "#4E79A7", "#F28E2B", "#E15759", "#76B7B2", "#59A14F", "#EDC948", "#B07AA1", "#FF9DA7",
    "#9C755F", "#BAB0AC",
];

#[derive(Clone, Debug, Default)]
pub struct HtmlOptions<'a> {
    pub community_labels: Option<&'a BTreeMap<usize, String>>,
    pub member_counts: Option<&'a BTreeMap<usize, usize>>,
    /// `Some` enables the Python-compatible aggregated fallback above the limit.
    pub node_limit: Option<isize>,
    pub learning_overlay: Option<&'a BTreeMap<String, Value>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HtmlRender {
    pub html: String,
    pub aggregated: bool,
    pub nodes: usize,
    pub edges: usize,
}

pub fn html_document(
    document: &GraphDocument,
    communities: &Communities,
    output_path: impl AsRef<Path>,
    options: &HtmlOptions<'_>,
) -> Result<Option<HtmlRender>, OutputError> {
    let limit = options.node_limit.unwrap_or_else(viz_node_limit);
    if (document.nodes.len() as isize) > limit {
        if options.node_limit.is_none() {
            return Err(OutputError::HtmlTooLarge {
                nodes: document.nodes.len(),
                limit,
            });
        }
        let (meta, meta_communities, member_counts) = aggregate(document, communities, options);
        if meta.nodes.len() <= 1 {
            return Ok(None);
        }
        let rendered = render(
            &meta,
            &meta_communities,
            output_path.as_ref(),
            &HtmlOptions {
                community_labels: options.community_labels,
                member_counts: Some(&member_counts),
                node_limit: None,
                learning_overlay: options.learning_overlay,
            },
            Some((document, communities)),
        );
        return Ok(Some(HtmlRender {
            nodes: meta.nodes.len(),
            edges: meta.links.len(),
            html: rendered,
            aggregated: true,
        }));
    }
    Ok(Some(HtmlRender {
        html: render(document, communities, output_path.as_ref(), options, None),
        aggregated: false,
        nodes: document.nodes.len(),
        edges: document.links.len(),
    }))
}

pub fn write_html(
    document: &GraphDocument,
    communities: &Communities,
    output_path: impl AsRef<Path>,
    options: &HtmlOptions<'_>,
) -> Result<Option<HtmlRender>, OutputError> {
    let output_path = output_path.as_ref();
    let owned_overlay;
    let effective = if options.learning_overlay.is_none() {
        owned_overlay = load_learning_overlay(output_path);
        HtmlOptions {
            community_labels: options.community_labels,
            member_counts: options.member_counts,
            node_limit: options.node_limit,
            learning_overlay: Some(&owned_overlay),
        }
    } else {
        options.clone()
    };
    let rendered = html_document(document, communities, output_path, &effective)?;
    if let Some(rendered) = &rendered {
        write_text_atomic(output_path, &rendered.html)?;
    }
    Ok(rendered)
}

fn render(
    document: &GraphDocument,
    communities: &Communities,
    output_path: &Path,
    options: &HtmlOptions<'_>,
    drilldown: Option<(&GraphDocument, &Communities)>,
) -> String {
    let nodes = node_values(document, communities, options);
    let edges = document.links.iter().map(edge_value).collect::<Vec<_>>();
    let mut legend = Vec::new();
    if let Some(labels) = options.community_labels {
        for community in labels.keys() {
            let count = options.member_counts.map_or_else(
                || communities.get(community).map(Vec::len).unwrap_or_default(),
                |counts| {
                    counts.get(community).copied().unwrap_or_else(|| {
                        communities.get(community).map(Vec::len).unwrap_or_default()
                    })
                },
            );
            let mut item = Map::new();
            item.insert("cid".into(), Value::from(*community));
            item.insert(
                "color".into(),
                Value::String(COMMUNITY_COLORS[community % COMMUNITY_COLORS.len()].into()),
            );
            item.insert(
                "label".into(),
                Value::String(html_escape(&sanitize_label(&community_name(
                    *community,
                    options.community_labels,
                )))),
            );
            item.insert("count".into(), Value::from(count));
            legend.push(Value::Object(item));
        }
    }
    let hyperedges = document
        .graph
        .get("hyperedges")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    let details = drilldown.map_or_else(
        || Value::Object(Map::new()),
        |(source, source_communities)| community_details(source, source_communities, options),
    );
    let nodes_json = js_safe(&python_json_compact(&Value::Array(nodes)));
    let edges_json = js_safe(&python_json_compact(&Value::Array(edges)));
    let legend_json = js_safe(&python_json_compact(&Value::Array(legend)));
    let hyperedges_json = js_safe(&python_json_compact(&hyperedges));
    let details_json = js_safe(&python_json_compact(&details));
    let title = html_escape(&sanitize_label(&output_path.to_string_lossy()));
    let stats = format!(
        "{} nodes &middot; {} edges &middot; {} communities",
        document.nodes.len(),
        document.links.len(),
        communities.len()
    );
    page(
        &title,
        &stats,
        &nodes_json,
        &edges_json,
        &legend_json,
        &hyperedges_json,
        &details_json,
        drilldown.is_some(),
    )
}

fn node_values(
    document: &GraphDocument,
    communities: &Communities,
    options: &HtmlOptions<'_>,
) -> Vec<Value> {
    let node_community = communities
        .iter()
        .flat_map(|(community, members)| {
            members
                .iter()
                .map(move |member| (member.as_str(), *community))
        })
        .collect::<HashMap<_, _>>();
    let degrees = degrees(document);
    let max_degree = degrees.values().copied().max().unwrap_or(1).max(1);
    let max_members = options
        .member_counts
        .and_then(|counts| counts.values().copied().max())
        .unwrap_or(1)
        .max(1);
    let mut nodes = Vec::new();
    for node in &document.nodes {
        let community = node_community.get(node.id.as_str()).copied().unwrap_or(0);
        let color = COMMUNITY_COLORS[community % COMMUNITY_COLORS.len()];
        let label = sanitize_label(&node_label(node));
        let degree = degrees.get(node.id.as_str()).copied().unwrap_or(1);
        let (size, font_size) = if let Some(counts) = options.member_counts {
            let count = counts.get(&community).copied().unwrap_or(1);
            (10.0 + 30.0 * count as f64 / max_members as f64, 12)
        } else {
            (
                10.0 + 30.0 * degree as f64 / max_degree as f64,
                if degree as f64 >= max_degree as f64 * 0.15 {
                    12
                } else {
                    0
                },
            )
        };
        let mut output = Map::new();
        output.insert("id".into(), Value::String(node.id.clone()));
        output.insert("label".into(), Value::String(label.clone()));
        output.insert("color".into(), node_color(color, color));
        output.insert("size".into(), decimal_value(round_tenths(size)));
        output.insert(
            "font".into(),
            serde_json::json!({"size": font_size, "color": "#ffffff"}),
        );
        output.insert("community".into(), Value::from(community));
        output.insert(
            "community_name".into(),
            Value::String(sanitize_label(&community_name(
                community,
                options.community_labels,
            ))),
        );
        output.insert(
            "source_file".into(),
            Value::String(sanitize_label(&node.string("source_file"))),
        );
        output.insert(
            "file_type".into(),
            node.attributes
                .get("file_type")
                .cloned()
                .unwrap_or_else(|| Value::String(String::new())),
        );
        let source_location = sanitize_label(&node.string("source_location"));
        let symbol_kind = sanitize_label(&node.string("symbol_kind"));
        let language = sanitize_label(&node.string("language"));
        let signature = sanitize_metadata(&node.string("signature"), 500);
        let (location_start, location_end) = source_line_range(&source_location);
        let line_start = node
            .attributes
            .get("line_start")
            .and_then(Value::as_u64)
            .or(location_start);
        let line_end = node
            .attributes
            .get("line_end")
            .and_then(Value::as_u64)
            .or(location_end)
            .or(line_start);
        let display_kind = if symbol_kind.is_empty() {
            sanitize_label(&node.string("file_type"))
        } else {
            symbol_kind
        };
        output.insert(
            "source_location".into(),
            Value::String(source_location.clone()),
        );
        output.insert("symbol_kind".into(), Value::String(display_kind.clone()));
        output.insert("language".into(), Value::String(language.clone()));
        output.insert(
            "line_start".into(),
            line_start.map_or(Value::Null, Value::from),
        );
        output.insert("line_end".into(), line_end.map_or(Value::Null, Value::from));
        output.insert("signature".into(), Value::String(signature.clone()));
        if let Some(counts) = options.member_counts {
            output.insert("is_community".into(), Value::Bool(true));
            output.insert(
                "member_count".into(),
                Value::from(counts.get(&community).copied().unwrap_or_default()),
            );
        }
        output.insert(
            "tooltip_html".into(),
            Value::String(node_tooltip(
                &label,
                &display_kind,
                &language,
                &node.string("source_file"),
                line_start,
                line_end,
                &signature,
                options
                    .member_counts
                    .and_then(|counts| counts.get(&community).copied()),
            )),
        );
        output.insert("degree".into(), Value::from(degree));
        if let Some(entry) = options
            .learning_overlay
            .filter(|overlay| !overlay.is_empty())
            .and_then(|overlay| overlay.get(&node.id))
            .and_then(Value::as_object)
        {
            add_learning_fields(&mut output, entry, &label, color);
        }
        output.remove("title");
        nodes.push(Value::Object(output));
    }
    nodes
}

fn community_details(
    document: &GraphDocument,
    communities: &Communities,
    options: &HtmlOptions<'_>,
) -> Value {
    let detail_options = HtmlOptions {
        community_labels: options.community_labels,
        member_counts: None,
        node_limit: None,
        learning_overlay: options.learning_overlay,
    };
    let mut grouped_nodes = BTreeMap::<usize, Vec<Value>>::new();
    for node in node_values(document, communities, &detail_options) {
        let community = node
            .get("community")
            .and_then(Value::as_u64)
            .unwrap_or_default() as usize;
        grouped_nodes.entry(community).or_default().push(node);
    }
    let node_community = communities
        .iter()
        .flat_map(|(community, members)| {
            members
                .iter()
                .map(move |member| (member.as_str(), *community))
        })
        .collect::<HashMap<_, _>>();
    let mut grouped_edges = BTreeMap::<usize, Vec<Value>>::new();
    for edge in &document.links {
        let (Some(source), Some(target)) = (
            node_community.get(edge.source.as_str()),
            node_community.get(edge.target.as_str()),
        ) else {
            continue;
        };
        if source == target {
            grouped_edges
                .entry(*source)
                .or_default()
                .push(edge_value(edge));
        }
    }
    Value::Object(
        grouped_nodes
            .into_iter()
            .map(|(community, nodes)| {
                let edges = grouped_edges.remove(&community).unwrap_or_default();
                (
                    community.to_string(),
                    serde_json::json!({
                        "community": community,
                        "name": community_name(community, options.community_labels),
                        "nodes": nodes,
                        "edges": edges,
                    }),
                )
            })
            .collect(),
    )
}

fn source_line_range(location: &str) -> (Option<u64>, Option<u64>) {
    let Some(location) = location.strip_prefix('L') else {
        return (None, None);
    };
    let (start, end) = location
        .split_once('-')
        .map_or((location, None), |(start, end)| (start, Some(end)));
    (start.parse().ok(), end.and_then(|end| end.parse().ok()))
}

fn node_tooltip(
    label: &str,
    symbol_kind: &str,
    language: &str,
    source_file: &str,
    line_start: Option<u64>,
    line_end: Option<u64>,
    signature: &str,
    member_count: Option<usize>,
) -> String {
    let kind = if symbol_kind.is_empty() {
        "symbol"
    } else {
        symbol_kind
    };
    let mut rows = Vec::new();
    if let Some(count) = member_count {
        rows.push(format!(
            "<span>{count} symbols · double-click to explore</span>"
        ));
    } else {
        if !language.is_empty() {
            rows.push(format!("<span>Language: {}</span>", html_escape(language)));
        }
        if !source_file.is_empty() {
            rows.push(format!(
                "<span class=\"hover-source\">{}</span>",
                html_escape(&sanitize_metadata(source_file, 320))
            ));
        }
        if let Some(start) = line_start {
            let range = line_end
                .filter(|end| *end != start)
                .map_or_else(|| start.to_string(), |end| format!("{start}–{end}"));
            rows.push(format!("<span>Lines: {range}</span>"));
        }
        if !signature.is_empty() {
            rows.push(format!(
                "<code>{}</code>",
                html_escape(&sanitize_metadata(signature, 320))
            ));
        }
    }
    format!(
        "<div class=\"node-hover-card\"><div><strong>{}</strong><b>{}</b></div>{}</div>",
        html_escape(label),
        html_escape(&kind.to_uppercase()),
        rows.join("")
    )
}

fn edge_value(edge: &EdgeRecord) -> Value {
    let confidence = defaulted(edge, "confidence", "EXTRACTED");
    let relation = edge.string("relation");
    let source = edge
        .attributes
        .get("_src")
        .and_then(Value::as_str)
        .unwrap_or(&edge.source);
    let target = edge
        .attributes
        .get("_tgt")
        .and_then(Value::as_str)
        .unwrap_or(&edge.target);
    let extracted = confidence == "EXTRACTED";
    let mut output = Map::new();
    output.insert("from".into(), Value::String(source.to_owned()));
    output.insert("to".into(), Value::String(target.to_owned()));
    output.insert("label".into(), Value::String(relation.clone()));
    output.insert(
        "title".into(),
        Value::String(html_escape(&format!("{relation} [{confidence}]"))),
    );
    output.insert("dashes".into(), Value::Bool(!extracted));
    output.insert("width".into(), Value::from(if extracted { 2 } else { 1 }));
    output.insert(
        "color".into(),
        serde_json::json!({"opacity": if extracted { 0.7 } else { 0.35 }}),
    );
    output.insert("confidence".into(), Value::String(confidence));
    Value::Object(output)
}

fn add_learning_fields(
    output: &mut Map<String, Value>,
    entry: &Map<String, Value>,
    label: &str,
    background: &str,
) {
    let status = sanitize_label(&python_string(entry.get("status")));
    let stale = entry.get("stale").and_then(Value::as_bool).unwrap_or(false);
    output.insert("learning_status".into(), Value::String(status.clone()));
    output.insert("learning_stale".into(), Value::Bool(stale));
    let ring = match status.as_str() {
        "preferred" => Some("#22c55e"),
        "contested" => Some("#f59e0b"),
        _ => None,
    };
    if let Some(mut ring) = ring {
        if stale {
            ring = "#9ca3af";
            output.insert(
                "shapeProperties".into(),
                serde_json::json!({"borderDashes":[4,4]}),
            );
        }
        output.insert("borderWidth".into(), Value::from(3));
        output.insert("color".into(), node_color(background, ring));
    }
    let uses = python_string(entry.get("uses"));
    let mut lesson = if status == "contested" {
        format!(
            "Lesson: contested (useful {uses} / dead-end {})",
            python_string(entry.get("neg"))
        )
    } else if status == "preferred" {
        format!(
            "Lesson: preferred source ({uses} useful, score={})",
            python_string(entry.get("score"))
        )
    } else {
        format!("Lesson: {status} ({uses} useful)")
    };
    if stale {
        lesson.push_str(" [code changed — re-verify]");
    }
    output.insert(
        "title".into(),
        Value::String(format!(
            "{}\n{}",
            html_escape(label),
            html_escape(&sanitize_label(&lesson))
        )),
    );
}

fn aggregate(
    document: &GraphDocument,
    communities: &Communities,
    options: &HtmlOptions<'_>,
) -> (GraphDocument, Communities, BTreeMap<usize, usize>) {
    let node_community = communities
        .iter()
        .flat_map(|(community, members)| {
            members
                .iter()
                .map(move |member| (member.as_str(), *community))
        })
        .collect::<HashMap<_, _>>();
    let nodes = communities
        .keys()
        .map(|community| NodeRecord {
            id: community.to_string(),
            attributes: Map::from_iter([
                (
                    "label".into(),
                    Value::String(community_name(*community, options.community_labels)),
                ),
                ("symbol_kind".into(), Value::String("community".to_owned())),
            ]),
        })
        .collect::<Vec<_>>();
    let mut counts = Vec::<((usize, usize), usize)>::new();
    let mut positions = HashMap::<(usize, usize), usize>::new();
    for edge in &document.links {
        let (Some(left), Some(right)) = (
            node_community.get(edge.source.as_str()),
            node_community.get(edge.target.as_str()),
        ) else {
            continue;
        };
        if left == right {
            continue;
        }
        let key = ((*left).min(*right), (*left).max(*right));
        if let Some(position) = positions.get(&key).copied() {
            counts[position].1 += 1;
        } else {
            positions.insert(key, counts.len());
            counts.push((key, 1));
        }
    }
    let links = counts
        .into_iter()
        .map(|((left, right), count)| EdgeRecord {
            source: left.to_string(),
            target: right.to_string(),
            attributes: Map::from_iter([
                ("weight".into(), Value::from(count)),
                (
                    "relation".into(),
                    Value::String(format!("{count} cross-community edges")),
                ),
                ("confidence".into(), Value::String("AGGREGATED".into())),
            ]),
        })
        .collect();
    let graph = remap_hyperedges(document, &node_community);
    let meta_communities = communities
        .keys()
        .map(|community| (*community, vec![community.to_string()]))
        .collect();
    let members = communities
        .iter()
        .map(|(community, nodes)| (*community, nodes.len()))
        .collect();
    (
        GraphDocument {
            directed: false,
            multigraph: false,
            graph,
            nodes,
            links,
            extras: BTreeMap::new(),
            used_legacy_edges_key: false,
        },
        meta_communities,
        members,
    )
}

fn remap_hyperedges(
    document: &GraphDocument,
    communities: &HashMap<&str, usize>,
) -> Map<String, Value> {
    let mut graph = Map::new();
    let mut output = Vec::new();
    let Some(hyperedges) = document.graph.get("hyperedges").and_then(Value::as_array) else {
        return graph;
    };
    for hyperedge in hyperedges {
        let Some(item) = hyperedge.as_object() else {
            continue;
        };
        let mut seen = Vec::new();
        for id in item
            .get("nodes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if let Some(community) = communities.get(id) {
                let id = community.to_string();
                if !seen.contains(&id) {
                    seen.push(id);
                }
            }
        }
        if seen.len() < 2 {
            continue;
        }
        output.push(serde_json::json!({
            "id": item.get("id").and_then(Value::as_str).unwrap_or_default(),
            "label": item.get("label").and_then(Value::as_str).filter(|label| !label.is_empty()).map_or_else(|| item.get("relation").and_then(Value::as_str).unwrap_or_default().replace('_', " "), ToOwned::to_owned),
            "nodes": seen,
        }));
    }
    if !output.is_empty() {
        graph.insert("hyperedges".into(), Value::Array(output));
    }
    graph
}

fn degrees(document: &GraphDocument) -> HashMap<&str, usize> {
    let mut degrees = document
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), 0))
        .collect::<HashMap<_, _>>();
    for edge in &document.links {
        *degrees.entry(edge.source.as_str()).or_default() += 1;
        *degrees.entry(edge.target.as_str()).or_default() += 1;
    }
    degrees
}

fn node_label(node: &NodeRecord) -> String {
    match node.attributes.get("label") {
        None => node.id.clone(),
        Some(Value::Null) => String::new(),
        Some(value) => python_value_string(value),
    }
}
fn python_string(value: Option<&Value>) -> String {
    value.map_or_else(|| "0".to_owned(), python_value_string)
}
fn python_value_string(value: &Value) -> String {
    match value {
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::String(value) => value.clone(),
        other => other.to_string(),
    }
}
fn defaulted(edge: &EdgeRecord, key: &str, default: &str) -> String {
    let value = edge.string(key);
    if value.is_empty() {
        default.into()
    } else {
        value
    }
}
fn community_name(community: usize, labels: Option<&BTreeMap<usize, String>>) -> String {
    labels
        .and_then(|labels| labels.get(&community).cloned())
        .unwrap_or_else(|| format!("Community {community}"))
}
fn sanitize_label(value: &str) -> String {
    sanitize_metadata(value, 256)
}
fn sanitize_metadata(value: &str, limit: usize) -> String {
    value
        .chars()
        .filter(|character| !((*character as u32) < 0x20 || *character == '\u{7f}'))
        .take(limit)
        .collect()
}
fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}
fn js_safe(value: &str) -> String {
    value.replace("</", "<\\/")
}
fn round_tenths(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}
fn decimal_value(value: f64) -> Value {
    serde_json::Number::from_f64(value).map_or(Value::Null, Value::Number)
}
fn node_color(background: &str, border: &str) -> Value {
    serde_json::json!({"background":background,"border":border,"highlight":{"background":"#ffffff","border":border}})
}

fn viz_node_limit() -> isize {
    std::env::var("GRAPHIFY_VIZ_NODE_LIMIT")
        .ok()
        .filter(|raw| !raw.trim().is_empty())
        .and_then(|raw| raw.trim().parse().ok())
        .unwrap_or(DEFAULT_NODE_LIMIT)
}

fn load_learning_overlay(output_path: &Path) -> BTreeMap<String, Value> {
    let path = output_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(".compass_learning.json");
    let raw = fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
    let Some(nodes) = raw
        .as_ref()
        .and_then(|value| value.get("nodes"))
        .and_then(Value::as_object)
    else {
        return BTreeMap::new();
    };
    nodes
        .iter()
        .filter_map(|(id, entry)| {
            let mut entry = entry.as_object()?.clone();
            entry.insert(
                "stale".into(),
                Value::Bool(learning_entry_is_stale(&entry, output_path)),
            );
            Some((id.clone(), Value::Object(entry)))
        })
        .collect()
}

fn learning_entry_is_stale(entry: &Map<String, Value>, output_path: &Path) -> bool {
    let source = entry
        .get("source_file")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if source.is_empty() {
        return false;
    }
    let Some(path) = resolve_learning_source(source, output_path) else {
        return true;
    };
    let stored = entry
        .get("code_fingerprint")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if stored.is_empty() {
        return true;
    }
    fs::read(path)
        .ok()
        .map(|bytes| format!("{:x}", Sha256::digest(bytes)))
        .is_none_or(|digest| digest != stored)
}

fn resolve_learning_source(source: &str, output_path: &Path) -> Option<std::path::PathBuf> {
    let source = Path::new(source);
    if source.is_absolute() {
        return source.is_file().then(|| source.to_path_buf());
    }
    let out = output_path.parent().unwrap_or_else(|| Path::new("."));
    let mut roots = Vec::new();
    if let Ok(recorded) = fs::read_to_string(out.join(".compass_root")) {
        let recorded = recorded.trim();
        if !recorded.is_empty() {
            roots.push(std::path::PathBuf::from(recorded));
        }
    }
    if out.file_name().and_then(|name| name.to_str()) == Some("compass-out") {
        if let Some(parent) = out.parent() {
            roots.push(parent.to_path_buf());
        }
        roots.push(out.to_path_buf());
    } else {
        roots.push(out.to_path_buf());
        if let Some(parent) = out.parent() {
            roots.push(parent.to_path_buf());
        }
    }
    if let Ok(current) = std::env::current_dir() {
        roots.push(current);
    }
    let mut seen = std::collections::HashSet::new();
    roots
        .into_iter()
        .filter(|root| seen.insert(root.clone()))
        .map(|root| root.join(source))
        .find(|candidate| candidate.is_file())
}

fn page(
    title: &str,
    stats: &str,
    nodes: &str,
    edges: &str,
    legend: &str,
    hyperedges: &str,
    details: &str,
    aggregated: bool,
) -> String {
    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Compass — {title}</title>
<script src="https://unpkg.com/vis-network@9.1.6/standalone/umd/vis-network.min.js" integrity="sha384-Ux6phic9PEHJ38YtrijhkzyJ8yQlH8i/+buBR8s3mAZOJrP1gwyvAcIYl3GWtpX1" crossorigin="anonymous"></script>
<style>
:root {{
  --canvas: #08111f;
  --canvas-deep: #050b14;
  --panel: #101b2d;
  --panel-raised: rgba(20, 34, 54, .88);
  --line: rgba(154, 178, 211, .16);
  --line-strong: rgba(154, 178, 211, .28);
  --text: #eef5ff;
  --muted: #91a4bd;
  --faint: #60728b;
  --focus: #76b7ff;
  --focus-soft: rgba(118, 183, 255, .12);
  --radius: 14px;
}}
* {{ box-sizing: border-box; margin: 0; padding: 0; }}
html, body {{ width: 100%; height: 100%; }}
body {{
  display: flex;
  overflow: hidden;
  background: var(--canvas);
  color: var(--text);
  font-family: Inter, ui-sans-serif, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  -webkit-font-smoothing: antialiased;
}}
button, input {{ font: inherit; }}
button {{ min-height: 40px; }}
.sr-only {{
  position: absolute;
  width: 1px;
  height: 1px;
  padding: 0;
  margin: -1px;
  overflow: hidden;
  clip: rect(0, 0, 0, 0);
  white-space: nowrap;
  border: 0;
}}
#workspace {{
  position: relative;
  flex: 1;
  min-width: 0;
  overflow: hidden;
}}
#graph {{
  width: 100%;
  height: 100%;
  background:
    radial-gradient(circle at 45% 42%, rgba(36, 73, 112, .34), transparent 34%),
    radial-gradient(circle at 76% 75%, rgba(43, 83, 95, .17), transparent 28%),
    linear-gradient(145deg, var(--canvas) 0%, var(--canvas-deep) 100%);
}}
#graph::after {{
  content: "";
  position: absolute;
  inset: 0;
  pointer-events: none;
  opacity: .16;
  background-image: radial-gradient(rgba(150, 180, 214, .4) .55px, transparent .55px);
  background-size: 22px 22px;
}}
.glass-panel {{
  background: var(--panel-raised);
  border: 1px solid var(--line);
  box-shadow: 0 18px 52px rgba(0, 0, 0, .3);
  backdrop-filter: blur(18px);
  -webkit-backdrop-filter: blur(18px);
}}
#graph-toolbar {{
  position: absolute;
  z-index: 4;
  top: 18px;
  left: 18px;
  right: 18px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  min-height: 58px;
  padding: 8px 10px 8px 16px;
  border-radius: var(--radius);
}}
#viewer-status,
.toolbar-actions,
#sidebar-header,
.node-identity,
.neighbors-heading,
.legend-item,
#legend-controls label {{
  display: flex;
  align-items: center;
}}
#viewer-status {{
  min-width: 0;
  gap: 10px;
  color: var(--muted);
  font-size: 12px;
  letter-spacing: .015em;
}}
#viewer-status-text {{
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}}
.status-dot {{
  width: 8px;
  height: 8px;
  flex: 0 0 auto;
  border-radius: 50%;
  background: var(--focus);
  box-shadow: 0 0 0 4px rgba(118, 183, 255, .1);
}}
#viewer-status[data-state="running"] .status-dot {{
  animation: status-pulse 1.6s ease-in-out infinite;
}}
@keyframes status-pulse {{
  50% {{ box-shadow: 0 0 0 8px rgba(118, 183, 255, 0); }}
}}
.toolbar-actions {{
  gap: 6px;
}}
.tool-button {{
  display: inline-flex;
  align-items: center;
  gap: 7px;
  padding: 0 12px;
  border: 1px solid transparent;
  border-radius: 10px;
  background: transparent;
  color: var(--muted);
  cursor: pointer;
  white-space: nowrap;
  transition: color .16s ease, background .16s ease, border-color .16s ease;
}}
.tool-button svg {{
  width: 15px;
  height: 15px;
  stroke: currentColor;
  stroke-width: 1.8;
  fill: none;
}}
.tool-button:hover,
.tool-button[aria-pressed="true"],
.tool-button.is-active {{
  color: var(--text);
  background: var(--focus-soft);
  border-color: var(--line);
}}
.tool-button[hidden] {{ display: none; }}
#node-tooltip,
.vis-tooltip {{
  max-width: 430px !important;
  padding: 0 !important;
  overflow: hidden;
  border: 1px solid var(--line-strong) !important;
  border-radius: 12px !important;
  background: #111c2a !important;
  color: var(--muted) !important;
  box-shadow: 0 18px 42px rgba(0, 0, 0, .42) !important;
  font-family: inherit !important;
}}
#node-tooltip {{
  position: absolute;
  z-index: 7;
  pointer-events: none;
}}
#node-tooltip[hidden] {{ display: none; }}
.node-hover-card {{
  display: grid;
  gap: 7px;
  padding: 13px 15px;
  font-size: 12px;
  line-height: 1.35;
}}
.node-hover-card > div {{
  display: flex;
  align-items: center;
  gap: 8px;
}}
.node-hover-card strong {{
  overflow: hidden;
  color: var(--text);
  font-size: 13px;
  text-overflow: ellipsis;
}}
.node-hover-card b,
.symbol-badge {{
  display: inline-flex;
  width: max-content;
  padding: 3px 7px;
  border-radius: 999px;
  background: rgba(89, 161, 79, .2);
  color: #8de384;
  font-size: 9px;
  font-weight: 800;
  letter-spacing: .07em;
}}
.node-hover-card span,
.node-hover-card code {{
  display: block;
}}
.node-hover-card .hover-source {{
  color: var(--focus);
}}
.node-hover-card code {{
  max-width: 395px;
  overflow: hidden;
  color: #cbd8e8;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  text-overflow: ellipsis;
  white-space: nowrap;
}}
#sidebar {{
  position: relative;
  z-index: 5;
  width: 340px;
  flex: 0 0 340px;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: linear-gradient(180deg, #111e31 0%, var(--panel) 100%);
  border-left: 1px solid var(--line);
  box-shadow: -22px 0 60px rgba(0, 0, 0, .18);
}}
#sidebar-header {{
  gap: 11px;
  min-height: 70px;
  padding: 14px 18px;
  border-bottom: 1px solid var(--line);
}}
.product-mark {{
  display: grid;
  place-items: center;
  width: 36px;
  height: 36px;
  border-radius: 11px;
  background: linear-gradient(145deg, #eff8ff, #a8d2ff);
  color: #09182c;
  box-shadow: inset 0 0 0 1px rgba(255, 255, 255, .48), 0 8px 22px rgba(60, 132, 202, .18);
  font-size: 15px;
  font-weight: 800;
}}
#sidebar-header strong,
#sidebar-header span {{
  display: block;
}}
#sidebar-header strong {{
  font-size: 14px;
  letter-spacing: .01em;
}}
#sidebar-header span {{
  margin-top: 2px;
  color: var(--faint);
  font-size: 11px;
}}
#search-wrap {{
  position: relative;
  padding: 14px 16px;
  border-bottom: 1px solid var(--line);
}}
.search-field {{
  position: relative;
}}
.search-field svg {{
  position: absolute;
  top: 50%;
  left: 13px;
  width: 15px;
  height: 15px;
  transform: translateY(-50%);
  fill: none;
  stroke: var(--faint);
  stroke-width: 1.8;
  pointer-events: none;
}}
#search {{
  width: 100%;
  min-height: 42px;
  padding: 0 13px 0 38px;
  border: 1px solid var(--line);
  border-radius: 11px;
  outline: none;
  background: rgba(5, 11, 20, .64);
  color: var(--text);
}}
#search::placeholder {{ color: var(--faint); }}
#search-results {{
  display: none;
  position: absolute;
  z-index: 8;
  top: 62px;
  left: 16px;
  right: 16px;
  max-height: 230px;
  overflow-y: auto;
  padding: 6px;
  border: 1px solid var(--line-strong);
  border-radius: 12px;
  background: #0d1929;
  box-shadow: 0 18px 38px rgba(0, 0, 0, .38);
}}
.search-item {{
  min-height: 38px;
  padding: 10px;
  overflow: hidden;
  border-left: 3px solid transparent;
  border-radius: 8px;
  color: var(--muted);
  font-size: 12px;
  text-overflow: ellipsis;
  white-space: nowrap;
  cursor: pointer;
}}
.search-item:hover,
.search-item[aria-selected="true"] {{
  background: var(--focus-soft);
  color: var(--text);
}}
#info-panel {{
  padding: 18px 16px;
  border-bottom: 1px solid var(--line);
}}
.section-heading {{
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  margin-bottom: 15px;
}}
.section-heading h2,
#legend-wrap h2 {{
  font-size: 11px;
  font-weight: 700;
  letter-spacing: .1em;
  text-transform: uppercase;
}}
.section-heading span {{
  color: var(--faint);
  font-size: 9px;
  letter-spacing: .1em;
  text-transform: uppercase;
}}
#info-content {{
  color: var(--muted);
  font-size: 12px;
  line-height: 1.45;
}}
.node-identity {{
  gap: 11px;
  margin-bottom: 15px;
}}
.node-identity > div {{
  min-width: 0;
}}
.node-identity strong,
.node-identity span {{
  display: block;
}}
.node-identity strong {{
  overflow: hidden;
  color: var(--text);
  font-size: 14px;
  text-overflow: ellipsis;
  white-space: nowrap;
}}
.node-identity span {{
  margin-top: 3px;
  color: var(--faint);
  font-size: 11px;
}}
.node-swatch {{
  width: 10px;
  height: 36px;
  flex: 0 0 auto;
  border-radius: 5px;
  background: var(--node-color);
}}
.metadata-grid {{
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 8px;
  margin-bottom: 15px;
}}
.metadata-grid > div {{
  min-width: 0;
  padding: 9px 10px;
  border: 1px solid var(--line);
  border-radius: 10px;
  background: rgba(5, 11, 20, .3);
}}
.metadata-grid .metadata-wide {{
  grid-column: 1 / -1;
}}
.metadata-grid dt {{
  color: var(--faint);
  font-size: 9px;
  letter-spacing: .08em;
  text-transform: uppercase;
}}
.metadata-grid dd {{
  margin-top: 4px;
  overflow: hidden;
  color: var(--text);
  text-overflow: ellipsis;
  white-space: nowrap;
}}
.signature-block {{
  margin-bottom: 15px;
  padding: 10px;
  overflow-wrap: anywhere;
  border: 1px solid var(--line);
  border-radius: 10px;
  background: rgba(5, 11, 20, .42);
  color: #cad8e9;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 11px;
}}
.inspector-actions {{
  display: flex;
  gap: 7px;
  margin-bottom: 14px;
}}
.inspector-action {{
  min-height: 34px;
  padding: 0 10px;
  border: 1px solid var(--line);
  border-radius: 9px;
  background: var(--focus-soft);
  color: var(--text);
  cursor: pointer;
}}
.inspector-action:hover {{ border-color: var(--line-strong); }}
.neighbors-heading {{
  justify-content: space-between;
  margin: 3px 0 7px;
  color: var(--muted);
}}
.neighbors-heading strong {{
  color: var(--faint);
  font-size: 10px;
}}
#neighbors-list {{
  max-height: 154px;
  overflow-y: auto;
}}
.neighbor-link {{
  display: block;
  width: 100%;
  min-height: 36px;
  margin: 3px 0;
  padding: 8px 9px;
  overflow: hidden;
  border: 0;
  border-left: 3px solid #334155;
  border-radius: 7px;
  background: transparent;
  color: var(--muted);
  text-align: left;
  text-overflow: ellipsis;
  white-space: nowrap;
  cursor: pointer;
}}
.neighbor-link:hover {{
  background: rgba(118, 183, 255, .08);
  color: var(--text);
}}
.empty {{
  display: block;
  padding: 8px 0;
  color: var(--faint);
}}
#legend-wrap {{
  flex: 1;
  overflow-y: auto;
  padding: 16px;
}}
#legend-wrap h2 {{
  margin-bottom: 10px;
}}
#legend-controls {{
  margin-bottom: 8px;
}}
#legend-controls label,
.legend-item {{
  min-height: 34px;
  gap: 8px;
  color: var(--muted);
  font-size: 12px;
  cursor: pointer;
}}
.legend-item {{
  border-radius: 7px;
}}
.legend-item:hover {{
  background: rgba(118, 183, 255, .06);
  color: var(--text);
}}
.legend-item.dimmed {{
  opacity: .36;
}}
.legend-dot {{
  width: 9px;
  height: 9px;
  flex: 0 0 auto;
  border-radius: 50%;
}}
.legend-label {{
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}}
.legend-count,
#stats {{
  color: var(--faint);
  font-size: 10px;
}}
#stats {{
  padding: 11px 16px;
  border-top: 1px solid var(--line);
  letter-spacing: .025em;
}}
.legend-cb,
#select-all-cb {{
  width: 15px;
  height: 15px;
  accent-color: var(--focus);
}}
.tool-button:focus-visible,
#search:focus-visible,
.neighbor-link:focus-visible,
.legend-item:focus-visible {{
  outline: 2px solid var(--focus);
  outline-offset: 2px;
}}
@media (max-width: 980px) {{
  .tool-button .button-label {{ display: none; }}
  .tool-button {{ padding: 0 11px; }}
}}
@media (max-width: 760px) {{
  body {{ display: block; }}
  #workspace {{ height: 62vh; min-height: 360px; }}
  #sidebar {{
    position: fixed;
    left: 0;
    right: 0;
    bottom: 0;
    width: 100%;
    height: 40vh;
    min-height: 280px;
    border-top: 1px solid var(--line);
    border-left: 0;
    border-radius: 18px 18px 0 0;
    box-shadow: 0 -22px 58px rgba(0, 0, 0, .35);
  }}
  #sidebar-header {{
    min-height: 58px;
    padding: 10px 16px;
  }}
  .product-mark {{ width: 32px; height: 32px; }}
  #graph-toolbar {{ top: 12px; left: 12px; right: 12px; }}
  .toolbar-actions {{ max-width: 62%; overflow-x: auto; }}
  #info-panel {{ padding-top: 14px; padding-bottom: 14px; }}
}}
@media (max-width: 480px) {{
  #graph-toolbar {{
    align-items: flex-start;
    flex-direction: column;
    gap: 7px;
  }}
  .toolbar-actions {{ width: 100%; max-width: none; }}
}}
@media (prefers-reduced-motion: reduce) {{
  *, *::before, *::after {{
    scroll-behavior: auto !important;
    transition-duration: .01ms !important;
    animation-duration: .01ms !important;
    animation-iteration-count: 1 !important;
  }}
}}
</style>
</head>
<body>
<main id="workspace">
  <div id="graph" role="region" aria-label="Interactive Compass knowledge graph"></div>
  <div id="node-tooltip" role="tooltip" hidden></div>
  <div id="graph-toolbar" class="glass-panel" role="toolbar" aria-label="Graph controls">
    <div id="viewer-status" data-state="running" role="status" aria-live="polite">
      <span class="status-dot" aria-hidden="true"></span>
      <span id="viewer-status-text">Layout settling</span>
    </div>
    <div class="toolbar-actions">
      <button id="back-overview" class="tool-button" type="button" aria-label="Back to community overview" hidden>
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m15 18-6-6 6-6"/></svg>
        <span class="button-label">Communities</span>
      </button>
      <button id="physics-toggle" class="tool-button is-active" type="button" aria-label="Pause layout" aria-pressed="true">
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M8 5v14M16 5v14"/></svg>
        <span class="button-label">Pause layout</span>
      </button>
      <button id="fit-graph" class="tool-button" type="button" aria-label="Fit graph in view">
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M8 3H3v5M16 3h5v5M8 21H3v-5M16 21h5v-5"/></svg>
        <span class="button-label">Fit graph</span>
      </button>
      <button id="reset-view" class="tool-button" type="button" aria-label="Reset graph view">
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 12a8 8 0 1 0 2.3-5.7L4 8.6M4 4v4.6h4.6"/></svg>
        <span class="button-label">Reset view</span>
      </button>
      <button id="labels-toggle" class="tool-button" type="button" aria-pressed="false">
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 5h16M8 5v14M5 19h6M14 10h6M17 10v9M14 19h6"/></svg>
        <span class="button-label">Show labels</span>
      </button>
    </div>
  </div>
</main>
<aside id="sidebar">
  <header id="sidebar-header">
    <div class="product-mark" aria-hidden="true">C</div>
    <div><strong>Compass</strong><span>Code knowledge map</span></div>
  </header>
  <div id="search-wrap" role="search">
    <label class="sr-only" for="search">Search graph nodes</label>
    <div class="search-field">
      <svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="11" cy="11" r="6.5"/><path d="m16 16 4 4"/></svg>
      <input id="search" type="search" placeholder="Search nodes…" autocomplete="off" aria-controls="search-results" aria-expanded="false" aria-autocomplete="list">
    </div>
    <div id="search-results" role="listbox" aria-label="Matching nodes"></div>
  </div>
  <section id="info-panel" aria-labelledby="info-title">
    <div class="section-heading"><h2 id="info-title">Inspector</h2><span id="info-mode">Node details</span></div>
    <div id="info-content"><span class="empty">Select a node to inspect its relationships</span></div>
  </section>
  <section id="legend-wrap" aria-labelledby="communities-title">
    <h2 id="communities-title">Communities</h2>
    <div id="legend-controls"><label><input type="checkbox" id="select-all-cb" checked> Select all</label></div>
    <div id="legend"></div>
  </section>
  <div id="stats">{stats}</div>
</aside>
<script>
const RAW_NODES = {nodes};
const RAW_EDGES = {edges};
const LEGEND = {legend};
const COMMUNITY_DETAILS = {details};
const IS_AGGREGATED = {aggregated};
const SEARCH_NODES = IS_AGGREGATED
  ? [
      ...RAW_NODES,
      ...Object.values(COMMUNITY_DETAILS).flatMap(detail => detail.nodes),
    ]
  : RAW_NODES;
let ACTIVE_NODES = RAW_NODES;
let ACTIVE_EDGES = RAW_EDGES;
function esc(value) {{
  return String(value)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}}

function decorateNode(node) {{
  return {{
  ...node,
  _baseColor: node.color,
  _baseFont: node.font,
  _baseBorderWidth: node.borderWidth || 1.5,
  _community: node.community,
  _community_name: node.community_name,
  _source_file: node.source_file,
  _source_location: node.source_location,
  _file_type: node.file_type,
  _symbol_kind: node.symbol_kind,
  _language: node.language,
  _line_start: node.line_start,
  _line_end: node.line_end,
  _signature: node.signature,
  _is_community: node.is_community,
  _member_count: node.member_count,
  _tooltip_html: node.tooltip_html,
  _degree: node.degree,
  }};
}}

function decorateEdge(edge, index) {{
  return {{
  id: index,
  from: edge.from,
  to: edge.to,
  label: '',
  title: edge.title,
  dashes: edge.dashes,
  width: edge.width,
  color: edge.color,
  _baseColor: edge.color,
  _baseWidth: edge.width,
  arrows: {{ to: {{ enabled: true, scaleFactor: .5 }} }},
  }};
}}

const nodesDS = new vis.DataSet(RAW_NODES.map(decorateNode));
const edgesDS = new vis.DataSet(RAW_EDGES.map(decorateEdge));
const container = document.getElementById('graph');
const network = new vis.Network(container, {{ nodes: nodesDS, edges: edgesDS }}, {{
  physics: {{
    enabled: true,
    solver: 'forceAtlas2Based',
    forceAtlas2Based: {{
      gravitationalConstant: -60,
      centralGravity: .005,
      springLength: 120,
      springConstant: .08,
      damping: .4,
      avoidOverlap: .8,
    }},
    stabilization: {{ iterations: 200, fit: true }},
  }},
  interaction: {{
    hover: true,
    tooltipDelay: 100,
    hideEdgesOnDrag: true,
    navigationButtons: false,
    keyboard: false,
  }},
  nodes: {{ shape: 'dot', borderWidth: 1.5 }},
  edges: {{ smooth: {{ type: 'continuous', roundness: .2 }}, selectionWidth: 3 }},
}});

const reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
const viewerState = {{
  physicsRunning: true,
  focusedNodeId: null,
  forceLabels: false,
  initialView: null,
  activeCommunity: null,
}};
const physicsToggle = document.getElementById('physics-toggle');
const labelsToggle = document.getElementById('labels-toggle');
const viewerStatus = document.getElementById('viewer-status');
const viewerStatusText = document.getElementById('viewer-status-text');
const backOverview = document.getElementById('back-overview');
const statsEl = document.getElementById('stats');
const overviewStats = statsEl.innerHTML;
const nodeTooltip = document.getElementById('node-tooltip');

function setViewerStatus(text) {{
  viewerStatusText.textContent = text;
}}

function hideNodeTooltip() {{
  nodeTooltip.hidden = true;
  nodeTooltip.replaceChildren();
}}

function showNodeTooltip(id) {{
  const node = nodesDS.get(id);
  const position = network.getPositions([id])[id];
  if (!node?._tooltip_html || !position) return;
  const point = network.canvasToDOM(position);
  nodeTooltip.innerHTML = node._tooltip_html;
  nodeTooltip.hidden = false;
  requestAnimationFrame(() => {{
    const left = Math.min(
      container.clientWidth - nodeTooltip.offsetWidth - 12,
      Math.max(12, point.x + 18)
    );
    const top = Math.min(
      container.clientHeight - nodeTooltip.offsetHeight - 12,
      Math.max(88, point.y - nodeTooltip.offsetHeight / 2)
    );
    nodeTooltip.style.left = `${{left}}px`;
    nodeTooltip.style.top = `${{top}}px`;
  }});
}}

function setPhysicsRunning(running) {{
  viewerState.physicsRunning = running;
  network.setOptions({{ physics: {{ enabled: running }} }});
  if (running) network.startSimulation();
  else network.stopSimulation();
  const label = running ? 'Pause layout' : 'Resume layout';
  physicsToggle.querySelector('.button-label').textContent = label;
  physicsToggle.setAttribute('aria-label', label);
  physicsToggle.setAttribute('aria-pressed', String(running));
  physicsToggle.classList.toggle('is-active', running);
  viewerStatus.dataset.state = running ? 'running' : 'paused';
  setViewerStatus(running ? 'Layout settling' : 'Layout paused');
}}

function applyRelationshipSpotlight(id) {{
  const neighbors = new Set(network.getConnectedNodes(id));
  const visible = new Set([id, ...neighbors]);
  nodesDS.update(nodesDS.get().map(node => ({{
    id: node.id,
    opacity: visible.has(node.id) ? 1 : .14,
    borderWidth: node.id === id ? Math.max(4, node._baseBorderWidth) : node._baseBorderWidth,
    shadow: node.id === id
      ? {{ enabled: true, color: node._baseColor.background, size: 24, x: 0, y: 0 }}
      : {{ enabled: false }},
  }})));
  edgesDS.update(edgesDS.get().map(edge => {{
    const connected = edge.from === id || edge.to === id;
    return {{
      id: edge.id,
      color: {{ ...edge._baseColor, opacity: connected ? .9 : .06 }},
      width: connected ? Math.max(2.5, edge._baseWidth) : edge._baseWidth,
    }};
  }}));
}}

function clearFocus() {{
  viewerState.focusedNodeId = null;
  network.unselectAll();
  nodesDS.update(nodesDS.get().map(node => ({{
    id: node.id,
    opacity: 1,
    borderWidth: node._baseBorderWidth,
    shadow: {{ enabled: false }},
  }})));
  edgesDS.update(edgesDS.get().map(edge => ({{
    id: edge.id,
    color: edge._baseColor,
    width: edge._baseWidth,
  }})));
  document.getElementById('info-content').innerHTML =
    '<span class="empty">Select a node to inspect its relationships</span>';
  document.getElementById('info-mode').textContent = 'Node details';
  viewerStatus.dataset.state = viewerState.physicsRunning ? 'running' : 'paused';
  setViewerStatus(viewerState.physicsRunning ? 'Layout settling' : 'Layout paused');
}}

function sourceRange(node) {{
  if (!node._line_start) return node._source_location || '';
  return node._line_end && node._line_end !== node._line_start
    ? `${{node._line_start}}–${{node._line_end}}`
    : String(node._line_start);
}}

function showInfo(id) {{
  const node = nodesDS.get(id);
  if (!node) return;
  const neighborIds = network.getConnectedNodes(id);
  const neighborItems = neighborIds.map(nid => {{
    const neighbor = nodesDS.get(nid);
    const color = neighbor ? neighbor._baseColor.background : '#334155';
    return `<button class="neighbor-link" type="button" style="border-left-color:${{esc(color)}}" data-nid="${{esc(nid)}}">${{esc(neighbor ? neighbor.label : nid)}}</button>`;
  }}).join('');
  const kind = node._symbol_kind || node._file_type || 'symbol';
  const range = sourceRange(node);
  const location = node._source_file
    ? `${{node._source_file}}${{range ? `:${{range}}` : ''}}`
    : '';
  const signature = node._signature
    ? `<div class="signature-block">${{esc(node._signature)}}</div>`
    : '';
  const actions = node._is_community
    ? `<div class="inspector-actions"><button class="inspector-action explore-community" type="button" data-community="${{esc(node._community)}}">Explore ${{node._member_count || 0}} symbols</button></div>`
    : location
      ? `<div class="inspector-actions"><button class="inspector-action copy-location" type="button" data-location="${{esc(location)}}">Copy file location</button></div>`
      : '';
  document.getElementById('info-content').innerHTML = `
    <div class="node-identity">
      <span class="node-swatch" style="--node-color:${{esc(node._baseColor.background)}}"></span>
      <div><strong>${{esc(node.label)}}</strong><span class="symbol-badge">${{esc(kind.toUpperCase())}}</span></div>
    </div>
    <dl class="metadata-grid">
      <div><dt>Community</dt><dd>${{esc(node._community_name)}}</dd></div>
      <div><dt>Degree</dt><dd>${{node._degree}}</dd></div>
      ${{node._language ? `<div><dt>Language</dt><dd>${{esc(node._language)}}</dd></div>` : ''}}
      ${{range ? `<div><dt>Lines</dt><dd>${{esc(range)}}</dd></div>` : ''}}
      <div class="metadata-wide"><dt>Source</dt><dd title="${{esc(node._source_file || 'Not recorded')}}">${{esc(node._source_file || 'Not recorded')}}</dd></div>
    </dl>
    ${{signature}}
    ${{actions}}
    ${{neighborIds.length
      ? `<div class="neighbors-heading"><span>Connected nodes</span><strong>${{neighborIds.length}}</strong></div><div id="neighbors-list">${{neighborItems}}</div>`
      : '<span class="empty">No connected nodes</span>'}}
  `;
  document.getElementById('info-mode').textContent = 'Pinned';
}}

function focusNode(id) {{
  const node = nodesDS.get(id);
  if (!node) return;
  setPhysicsRunning(false);
  viewerState.focusedNodeId = id;
  applyRelationshipSpotlight(id);
  network.selectNodes([id]);
  network.focus(id, {{
    scale: 1.35,
    animation: reduceMotion ? false : {{ duration: 260, easingFunction: 'easeInOutQuad' }},
  }});
  showInfo(id);
  viewerStatus.dataset.state = 'inspecting';
  setViewerStatus(`Inspecting ${{node.label}}`);
}}

function replaceGraph(nodes, edges) {{
  clearFocus();
  ACTIVE_NODES = nodes;
  ACTIVE_EDGES = edges;
  nodesDS.clear();
  edgesDS.clear();
  nodesDS.add(nodes.map(decorateNode));
  edgesDS.add(edges.map(decorateEdge));
  if (viewerState.forceLabels) {{
    nodesDS.update(nodes.map(node => ({{
      id: node.id,
      font: {{ ...node.font, size: 12 }},
    }})));
  }}
  setPhysicsRunning(true);
  network.once('stabilizationIterationsDone', () => {{
    setPhysicsRunning(false);
    network.fit({{ animation: false }});
  }});
  network.stabilize(180);
}}

function enterCommunity(community, focusId = null) {{
  const key = String(community);
  const detail = COMMUNITY_DETAILS[key];
  if (!detail) return;
  viewerState.activeCommunity = Number(community);
  backOverview.hidden = false;
  document.getElementById('communities-title').textContent = detail.name;
  statsEl.textContent = `${{detail.nodes.length}} symbols · ${{detail.edges.length}} internal edges`;
  replaceGraph(detail.nodes, detail.edges);
  setViewerStatus(`Exploring ${{detail.name}}`);
  if (focusId !== null) requestAnimationFrame(() => focusNode(focusId));
}}

function exitCommunity() {{
  if (viewerState.activeCommunity === null) return;
  viewerState.activeCommunity = null;
  backOverview.hidden = true;
  document.getElementById('communities-title').textContent = 'Communities';
  statsEl.innerHTML = overviewStats;
  replaceGraph(RAW_NODES, RAW_EDGES);
  setViewerStatus('Community overview');
}}

async function copyText(value) {{
  if (navigator.clipboard?.writeText) {{
    await navigator.clipboard.writeText(value);
    return;
  }}
  const field = document.createElement('textarea');
  field.value = value;
  field.setAttribute('readonly', '');
  field.style.position = 'fixed';
  field.style.opacity = '0';
  document.body.appendChild(field);
  field.select();
  document.execCommand('copy');
  field.remove();
}}

physicsToggle.addEventListener('click', () => {{
  setPhysicsRunning(!viewerState.physicsRunning);
}});
backOverview.addEventListener('click', exitCommunity);
document.getElementById('fit-graph').addEventListener('click', () => {{
  network.fit({{
    animation: reduceMotion ? false : {{ duration: 280, easingFunction: 'easeInOutQuad' }},
  }});
}});
document.getElementById('reset-view').addEventListener('click', () => {{
  clearFocus();
  if (viewerState.initialView) {{
    network.moveTo({{
      position: viewerState.initialView.position,
      scale: viewerState.initialView.scale,
      animation: reduceMotion ? false : {{ duration: 280, easingFunction: 'easeInOutQuad' }},
    }});
  }} else {{
    network.fit({{ animation: false }});
  }}
}});
labelsToggle.addEventListener('click', () => {{
  viewerState.forceLabels = !viewerState.forceLabels;
  labelsToggle.setAttribute('aria-pressed', String(viewerState.forceLabels));
  labelsToggle.querySelector('.button-label').textContent =
    viewerState.forceLabels ? 'Hide labels' : 'Show labels';
  nodesDS.update(ACTIVE_NODES.map(node => ({{
    id: node.id,
    font: {{ ...node.font, size: viewerState.forceLabels ? 12 : node.font.size }},
  }})));
}});

network.once('stabilizationIterationsDone', () => {{
  setPhysicsRunning(false);
  viewerState.initialView = {{
    position: network.getViewPosition(),
    scale: network.getScale(),
  }};
}});
network.on('click', params => {{
  hideNodeTooltip();
  if (params.nodes.length) focusNode(params.nodes[0]);
  else clearFocus();
}});
network.on('hoverNode', params => showNodeTooltip(params.node));
network.on('blurNode', hideNodeTooltip);
network.on('dragStart', hideNodeTooltip);
network.on('zoom', hideNodeTooltip);
network.on('doubleClick', params => {{
  if (!IS_AGGREGATED || !params.nodes.length) return;
  const node = nodesDS.get(params.nodes[0]);
  if (node?._is_community) enterCommunity(node._community);
}});
document.addEventListener('click', async event => {{
  const link = event.target.closest('.neighbor-link');
  if (link && link.dataset.nid !== undefined) focusNode(link.dataset.nid);
  const explore = event.target.closest('.explore-community');
  if (explore?.dataset.community !== undefined) enterCommunity(explore.dataset.community);
  const copy = event.target.closest('.copy-location');
  if (copy?.dataset.location) {{
    try {{
      await copyText(copy.dataset.location);
      copy.textContent = 'Copied';
    }} catch {{
      copy.textContent = 'Copy failed';
    }}
  }}
}});

const results = document.getElementById('search-results');
const search = document.getElementById('search');
let searchMatches = [];
let activeSearchIndex = -1;

function closeSearchResults() {{
  results.replaceChildren();
  results.style.display = 'none';
  search.setAttribute('aria-expanded', 'false');
  search.removeAttribute('aria-activedescendant');
  activeSearchIndex = -1;
}}

function chooseSearchResult(node) {{
  if (IS_AGGREGATED && !node.is_community
      && viewerState.activeCommunity !== Number(node.community)) {{
    enterCommunity(node.community, node.id);
  }} else {{
    focusNode(node.id);
  }}
  search.value = '';
  closeSearchResults();
  search.focus();
}}

function renderSearchResults() {{
  results.replaceChildren();
  searchMatches.forEach((node, index) => {{
    const option = document.createElement('div');
    option.id = `search-option-${{index}}`;
    option.className = 'search-item';
    option.setAttribute('role', 'option');
    option.setAttribute('aria-selected', String(index === activeSearchIndex));
    option.textContent = node.source_file
      ? `${{node.label}} — ${{node.source_file}}`
      : node.label;
    option.style.borderLeftColor = node.color.background;
    option.addEventListener('click', () => chooseSearchResult(node));
    results.appendChild(option);
  }});
  const open = searchMatches.length > 0;
  results.style.display = open ? 'block' : 'none';
  search.setAttribute('aria-expanded', String(open));
  if (activeSearchIndex >= 0) {{
    const optionId = `search-option-${{activeSearchIndex}}`;
    search.setAttribute('aria-activedescendant', optionId);
    document.getElementById(optionId)?.scrollIntoView({{ block: 'nearest' }});
  }} else {{
    search.removeAttribute('aria-activedescendant');
  }}
}}

search.addEventListener('input', () => {{
  const query = search.value.toLowerCase().trim();
  searchMatches = query
    ? SEARCH_NODES.filter(node =>
        `${{node.label}} ${{node.source_file || ''}} ${{node.signature || ''}}`
          .toLowerCase()
          .includes(query)
      ).slice(0, 20)
    : [];
  activeSearchIndex = searchMatches.length ? 0 : -1;
  renderSearchResults();
}});
search.addEventListener('keydown', event => {{
  if (!searchMatches.length && event.key !== 'Escape') return;
  switch (event.key) {{
    case 'ArrowDown':
      event.preventDefault();
      activeSearchIndex = (activeSearchIndex + 1) % searchMatches.length;
      renderSearchResults();
      break;
    case 'ArrowUp':
      event.preventDefault();
      activeSearchIndex = (activeSearchIndex - 1 + searchMatches.length) % searchMatches.length;
      renderSearchResults();
      break;
    case 'Enter':
      if (activeSearchIndex >= 0) {{
        event.preventDefault();
        chooseSearchResult(searchMatches[activeSearchIndex]);
      }}
      break;
    case 'Escape':
      closeSearchResults();
      break;
  }}
}});
document.addEventListener('click', event => {{
  if (!results.contains(event.target) && event.target !== search) closeSearchResults();
}});

const hiddenCommunities = new Set();
const selectAll = document.getElementById('select-all-cb');

function updateVisibility() {{
  nodesDS.update(ACTIVE_NODES.map(node => ({{
    id: node.id,
    hidden: hiddenCommunities.has(node.community),
  }})));
}}

function updateSelectAllState() {{
  selectAll.checked = hiddenCommunities.size === 0;
  selectAll.indeterminate =
    hiddenCommunities.size > 0 && hiddenCommunities.size < LEGEND.length;
}}

function toggleAllCommunities(hide) {{
  LEGEND.forEach(community => {{
    if (hide) hiddenCommunities.add(community.cid);
    else hiddenCommunities.delete(community.cid);
  }});
  document.querySelectorAll('.legend-cb').forEach(checkbox => {{
    checkbox.checked = !hide;
  }});
  document.querySelectorAll('.legend-item').forEach(item => {{
    item.classList.toggle('dimmed', hide);
  }});
  updateVisibility();
  updateSelectAllState();
}}

selectAll.addEventListener('change', () => {{
  toggleAllCommunities(!selectAll.checked);
}});
const legendEl = document.getElementById('legend');
LEGEND.forEach(community => {{
  const item = document.createElement('label');
  item.className = 'legend-item';
  const checkbox = document.createElement('input');
  checkbox.type = 'checkbox';
  checkbox.className = 'legend-cb';
  checkbox.checked = true;
  const dot = document.createElement('span');
  dot.className = 'legend-dot';
  dot.style.background = community.color;
  const label = document.createElement('span');
  label.className = 'legend-label';
  // Legend labels are HTML-escaped in Rust before serialization. Assigning the
  // escaped value as markup decodes entities while keeping source text inert.
  label.innerHTML = community.label;
  const count = document.createElement('span');
  count.className = 'legend-count';
  count.textContent = community.count;
  checkbox.addEventListener('change', () => {{
    if (checkbox.checked) hiddenCommunities.delete(community.cid);
    else hiddenCommunities.add(community.cid);
    item.classList.toggle('dimmed', !checkbox.checked);
    updateVisibility();
    updateSelectAllState();
  }});
  item.append(checkbox, dot, label, count);
  legendEl.appendChild(item);
}});
</script>
<script>
const hyperedges = {hyperedges};
network.on('afterDrawing', ctx => {{
  if (viewerState.activeCommunity !== null) return;
  hyperedges.forEach(hyperedge => {{
    const positions = hyperedge.nodes
      .map(id => network.getPositions([id])[id])
      .filter(Boolean);
    if (positions.length < 2) return;
    const centerX = positions.reduce((sum, point) => sum + point.x, 0) / positions.length;
    const centerY = positions.reduce((sum, point) => sum + point.y, 0) / positions.length;
    const expanded = positions.map(point => ({{
      x: centerX + (point.x - centerX) * 1.15,
      y: centerY + (point.y - centerY) * 1.15,
    }}));
    ctx.save();
    ctx.globalAlpha = .1;
    ctx.fillStyle = '#76b7ff';
    ctx.strokeStyle = '#76b7ff';
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    ctx.moveTo(expanded[0].x, expanded[0].y);
    expanded.slice(1).forEach(point => ctx.lineTo(point.x, point.y));
    ctx.closePath();
    ctx.fill();
    ctx.globalAlpha = .24;
    ctx.stroke();
    ctx.restore();
  }});
}});
</script>
</body>
</html>"##
    )
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use serde_json::json;
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn script_data_cannot_close_its_script_tag() -> Result<(), Box<dyn Error>> {
        let graph: GraphDocument = serde_json::from_value(json!({
            "nodes":[{"id":"bad\" onmouseover=\"x", "label":"</script><script>alert(1)</script>"}],
            "links":[]
        }))?;
        let rendered = html_document(
            &graph,
            &Communities::new(),
            "graph.html",
            &HtmlOptions::default(),
        )?
        .ok_or("HTML unexpectedly skipped")?;
        assert!(rendered.html.contains("<\\/script>"));
        assert!(!rendered.html.contains("onclick=\"focusNode("));
        assert!(rendered.html.contains("data-nid=\"${esc(nid)}\""));
        Ok(())
    }

    #[test]
    fn html_exposes_stable_layout_controls() -> Result<(), Box<dyn Error>> {
        let graph: GraphDocument = serde_json::from_value(json!({
            "nodes":[{"id":"a","label":"A"},{"id":"b","label":"B"}],
            "links":[{"source":"a","target":"b","relation":"calls"}]
        }))?;
        let rendered = html_document(
            &graph,
            &Communities::new(),
            "graph.html",
            &HtmlOptions::default(),
        )?
        .ok_or("HTML unexpectedly skipped")?;
        for marker in [
            "id=\"graph-toolbar\"",
            "id=\"physics-toggle\"",
            "id=\"fit-graph\"",
            "id=\"reset-view\"",
            "id=\"labels-toggle\"",
            "id=\"viewer-status\"",
            "const viewerState =",
            "function setPhysicsRunning(running)",
            "network.stopSimulation()",
            "network.once('stabilizationIterationsDone'",
            "function applyRelationshipSpotlight(id)",
            "function clearFocus()",
            "function focusNode(id)",
            "setPhysicsRunning(false);",
            "applyRelationshipSpotlight(id);",
            "focusNode(params.nodes[0]);",
            "else clearFocus();",
        ] {
            assert!(rendered.html.contains(marker), "missing {marker}");
        }
        Ok(())
    }

    #[test]
    fn html_inspector_is_accessible_and_responsive() -> Result<(), Box<dyn Error>> {
        let graph: GraphDocument = serde_json::from_value(json!({
            "nodes":[{"id":"a","label":"A"}],
            "links":[]
        }))?;
        let rendered = html_document(
            &graph,
            &Communities::new(),
            "graph.html",
            &HtmlOptions::default(),
        )?
        .ok_or("HTML unexpectedly skipped")?;
        for marker in [
            "<strong>Compass</strong>",
            "role=\"search\"",
            "role=\"listbox\"",
            "aria-controls=\"search-results\"",
            "search.addEventListener('keydown'",
            "case 'ArrowDown':",
            "case 'ArrowUp':",
            "case 'Enter':",
            "case 'Escape':",
            "@media (max-width: 760px)",
            "@media (prefers-reduced-motion: reduce)",
            ":focus-visible",
            "class=\"node-identity\"",
            "class=\"metadata-grid\"",
        ] {
            assert!(rendered.html.contains(marker), "missing {marker}");
        }
        Ok(())
    }

    #[test]
    fn explicit_limit_builds_community_meta_graph() -> Result<(), Box<dyn Error>> {
        let graph: GraphDocument = serde_json::from_value(json!({
            "nodes":[
                {
                    "id":"a","label":"A()","source_file":"src/a.py",
                    "source_location":"L4","line_start":4,"line_end":8,
                    "symbol_kind":"function","language":"python",
                    "signature":"def A(value)"
                },
                {"id":"b","label":"B"},
                {"id":"c","label":"C"},{"id":"d","label":"D"}
            ],
            "links":[
                {"source":"a","target":"b"},
                {"source":"a","target":"c"},
                {"source":"b","target":"d"}
            ]
        }))?;
        let communities = BTreeMap::from([
            (0, vec!["a".into(), "b".into()]),
            (1, vec!["c".into(), "d".into()]),
        ]);
        let rendered = html_document(
            &graph,
            &communities,
            "graph.html",
            &HtmlOptions {
                node_limit: Some(2),
                ..HtmlOptions::default()
            },
        )?
        .ok_or("aggregated HTML unexpectedly skipped")?;
        assert!(rendered.aggregated);
        assert_eq!((rendered.nodes, rendered.edges), (2, 1));
        assert!(rendered.html.contains("2 cross-community edges"));
        for marker in [
            "const COMMUNITY_DETAILS =",
            "const IS_AGGREGATED = true",
            "function enterCommunity(community, focusId = null)",
            "id=\"back-overview\"",
            "\"symbol_kind\": \"function\"",
            "\"language\": \"python\"",
            "\"line_start\": 4",
            "\"line_end\": 8",
            "\"signature\": \"def A(value)\"",
            "class=\\\"node-hover-card\\\"",
        ] {
            assert!(rendered.html.contains(marker), "missing {marker}");
        }
        Ok(())
    }

    #[test]
    fn sidecar_overlay_is_loaded_and_staleness_recomputed() -> Result<(), Box<dyn Error>> {
        let directory = tempdir()?;
        let out = directory.path().join("compass-out");
        fs::create_dir(&out)?;
        fs::write(directory.path().join("source.rs"), "fn main() {}")?;
        let digest = format!(
            "{:x}",
            Sha256::digest(fs::read(directory.path().join("source.rs"))?)
        );
        fs::write(
            out.join(".compass_learning.json"),
            serde_json::to_vec(&json!({"nodes":{"a":{
                "status":"preferred","uses":2,"score":1.5,
                "source_file":"source.rs","code_fingerprint":digest
            }}}))?,
        )?;
        let graph: GraphDocument = serde_json::from_value(json!({
            "nodes":[{"id":"a","label":"A"}],"links":[]
        }))?;
        let rendered = write_html(
            &graph,
            &Communities::new(),
            out.join("graph.html"),
            &HtmlOptions::default(),
        )?
        .ok_or("HTML unexpectedly skipped")?;
        assert!(rendered.html.contains("\"learning_status\": \"preferred\""));
        assert!(rendered.html.contains("\"learning_stale\": false"));
        Ok(())
    }

    #[test]
    fn scalar_labels_hyperedges_and_learning_staleness_cover_boundary_shapes()
    -> Result<(), Box<dyn Error>> {
        let node: NodeRecord = serde_json::from_value(json!({"id":"fallback"}))?;
        assert_eq!(node_label(&node), "fallback");
        let null_label: NodeRecord = serde_json::from_value(json!({"id":"id","label":null}))?;
        assert_eq!(node_label(&null_label), "");
        assert_eq!(python_string(None), "0");
        assert_eq!(python_string(Some(&Value::Null)), "None");
        assert_eq!(python_value_string(&Value::Null), "None");
        assert_eq!(python_value_string(&Value::Bool(true)), "True");
        assert_eq!(python_value_string(&Value::Bool(false)), "False");
        assert_eq!(python_value_string(&json!(7)), "7");
        assert_eq!(python_value_string(&json!([1])), "[1]");
        assert_eq!(sanitize_label("<tag>\nline"), "<tag>line");
        assert_eq!(html_escape("&\"'"), "&amp;&quot;&#x27;");
        assert_eq!(js_safe("</script>"), "<\\/script>");

        let edge: EdgeRecord = serde_json::from_value(json!({
            "source":"a","target":"b","relation":null,"weight":false
        }))?;
        assert_eq!(defaulted(&edge, "missing", "fallback"), "fallback");
        assert_eq!(defaulted(&edge, "relation", "fallback"), "fallback");
        assert_eq!(defaulted(&edge, "weight", "fallback"), "False");

        let document: GraphDocument = serde_json::from_value(json!({
            "graph":{"hyperedges":[
                7,
                {"id":"single","nodes":["a"]},
                {"id":"cross","relation":"works_with","nodes":["a","b","c","b"]}
            ]},
            "nodes":[],"links":[]
        }))?;
        let communities = HashMap::from([("a", 0_usize), ("b", 1_usize), ("c", 1_usize)]);
        let remapped = remap_hyperedges(&document, &communities);
        assert_eq!(remapped["hyperedges"].as_array().map(Vec::len), Some(1));
        assert_eq!(remapped["hyperedges"][0]["label"], "works with");
        assert_eq!(remapped["hyperedges"][0]["nodes"], json!(["0", "1"]));

        let directory = tempdir()?;
        let output = directory.path().join("custom/graph.html");
        fs::create_dir_all(output.parent().ok_or("missing parent")?)?;
        let mut entry = Map::new();
        assert!(!learning_entry_is_stale(&entry, &output));
        entry.insert(
            "source_file".to_owned(),
            Value::String("missing.rs".to_owned()),
        );
        assert!(learning_entry_is_stale(&entry, &output));
        let source = directory.path().join("source.rs");
        fs::write(&source, "fn source() {}")?;
        entry.insert(
            "source_file".to_owned(),
            Value::String(source.to_string_lossy().into_owned()),
        );
        entry.insert("code_fingerprint".to_owned(), Value::String(String::new()));
        assert!(learning_entry_is_stale(&entry, &output));
        assert_eq!(resolve_learning_source("", &output), None);
        Ok(())
    }
}
