use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use trail_files::write_text_atomic;
use trail_graph::Communities;
use trail_model::{EdgeRecord, GraphDocument, NodeRecord};

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
        );
        return Ok(Some(HtmlRender {
            nodes: meta.nodes.len(),
            edges: meta.links.len(),
            html: rendered,
            aggregated: true,
        }));
    }
    Ok(Some(HtmlRender {
        html: render(document, communities, output_path.as_ref(), options),
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
) -> String {
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
        output.insert("title".into(), Value::String(html_escape(&label)));
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
        output.insert("degree".into(), Value::from(degree));
        if let Some(entry) = options
            .learning_overlay
            .filter(|overlay| !overlay.is_empty())
            .and_then(|overlay| overlay.get(&node.id))
            .and_then(Value::as_object)
        {
            add_learning_fields(&mut output, entry, &label, color);
        }
        nodes.push(Value::Object(output));
    }

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
    let nodes_json = js_safe(&python_json_compact(&Value::Array(nodes)));
    let edges_json = js_safe(&python_json_compact(&Value::Array(edges)));
    let legend_json = js_safe(&python_json_compact(&Value::Array(legend)));
    let hyperedges_json = js_safe(&python_json_compact(&hyperedges));
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
            attributes: Map::from_iter([(
                "label".into(),
                Value::String(community_name(*community, options.community_labels)),
            )]),
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
    value
        .chars()
        .filter(|character| !((*character as u32) < 0x20 || *character == '\u{7f}'))
        .take(256)
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
        .join(".graphify_learning.json");
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
    if let Ok(recorded) = fs::read_to_string(out.join(".graphify_root")) {
        let recorded = recorded.trim();
        if !recorded.is_empty() {
            roots.push(std::path::PathBuf::from(recorded));
        }
    }
    if out.file_name().and_then(|name| name.to_str()) == Some("graphify-out") {
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
) -> String {
    format!(
        r##"<!DOCTYPE html>
<html lang="en"><head><meta charset="UTF-8"><title>graphify - {title}</title>
<script src="https://unpkg.com/vis-network@9.1.6/standalone/umd/vis-network.min.js" integrity="sha384-Ux6phic9PEHJ38YtrijhkzyJ8yQlH8i/+buBR8s3mAZOJrP1gwyvAcIYl3GWtpX1" crossorigin="anonymous"></script>
<style>*{{box-sizing:border-box}}body{{margin:0;background:#0f0f1a;color:#e0e0e0;font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;display:flex;height:100vh;overflow:hidden}}#graph{{flex:1}}#sidebar{{width:280px;background:#1a1a2e;border-left:1px solid #2a2a4e;display:flex;flex-direction:column;overflow:hidden}}#search-wrap,#info-panel,#legend-wrap,#stats{{padding:12px}}#search{{width:100%;background:#0f0f1a;border:1px solid #3a3a5e;color:#e0e0e0;padding:7px 10px;border-radius:6px}}#search-results,#neighbors-list{{max-height:160px;overflow:auto}}.search-item,.neighbor-link,.legend-item{{padding:4px 6px;cursor:pointer;border-radius:4px;font-size:12px}}.neighbor-link{{display:block;border-left:3px solid #333}}.legend-item{{display:flex;gap:8px;align-items:center}}.legend-dot{{width:12px;height:12px;border-radius:50%}}.dimmed{{opacity:.35}}#legend-wrap{{flex:1;overflow:auto}}#stats{{color:#777}}</style></head>
<body><div id="graph"></div><div id="sidebar"><div id="search-wrap"><input id="search" placeholder="Search nodes..." autocomplete="off"><div id="search-results"></div></div><div id="info-panel"><h3>Node Info</h3><div id="info-content"><span class="empty">Click a node to inspect it</span></div></div><div id="legend-wrap"><h3>Communities</h3><label><input type="checkbox" id="select-all-cb" checked onchange="toggleAllCommunities(!this.checked)">Select All</label><div id="legend"></div></div><div id="stats">{stats}</div></div>
<script>
const RAW_NODES = {nodes};
const RAW_EDGES = {edges};
const LEGEND = {legend};
function esc(s){{return String(s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;').replace(/'/g,'&#39;')}}
const nodesDS=new vis.DataSet(RAW_NODES.map(n=>({{...n,_community:n.community,_community_name:n.community_name,_source_file:n.source_file,_file_type:n.file_type,_degree:n.degree}})));
const edgesDS=new vis.DataSet(RAW_EDGES.map((e,i)=>({{id:i,from:e.from,to:e.to,label:'',title:e.title,dashes:e.dashes,width:e.width,color:e.color,arrows:{{to:{{enabled:true,scaleFactor:.5}}}}}})));
const container=document.getElementById('graph');const network=new vis.Network(container,{{nodes:nodesDS,edges:edgesDS}},{{physics:{{solver:'forceAtlas2Based',stabilization:{{iterations:200}}}},interaction:{{hover:true,tooltipDelay:100,hideEdgesOnDrag:true}},nodes:{{shape:'dot'}},edges:{{smooth:{{type:'continuous'}}}}}});
function showInfo(id){{const n=nodesDS.get(id);if(!n)return;const ids=network.getConnectedNodes(id);const links=ids.map(nid=>{{const nb=nodesDS.get(nid);return `<span class="neighbor-link" data-nid="${{esc(nid)}}">${{esc(nb?nb.label:nid)}}</span>`}}).join('');document.getElementById('info-content').innerHTML=`<b>${{esc(n.label)}}</b><div>Type: ${{esc(n._file_type||'unknown')}}</div><div>Community: ${{esc(n._community_name)}}</div><div>Source: ${{esc(n._source_file||'-')}}</div><div>Degree: ${{n._degree}}</div><div id="neighbors-list">${{links}}</div>`}}
function focusNode(id){{network.focus(id,{{scale:1.4,animation:true}});network.selectNodes([id]);showInfo(id)}}
document.addEventListener('click',e=>{{const el=e.target.closest('.neighbor-link');if(el&&el.dataset.nid!==undefined)focusNode(el.dataset.nid)}});network.on('click',p=>{{if(p.nodes.length)showInfo(p.nodes[0])}});
const results=document.getElementById('search-results'),search=document.getElementById('search');search.addEventListener('input',()=>{{results.innerHTML='';const q=search.value.toLowerCase().trim();RAW_NODES.filter(n=>n.label.toLowerCase().includes(q)).slice(0,20).forEach(n=>{{const el=document.createElement('div');el.className='search-item';el.textContent=n.label;el.onclick=()=>focusNode(n.id);results.appendChild(el)}})}});
const hiddenCommunities=new Set();function updateVisibility(){{nodesDS.update(RAW_NODES.map(n=>({{id:n.id,hidden:hiddenCommunities.has(n.community)}})))}}function toggleAllCommunities(hide){{LEGEND.forEach(c=>hide?hiddenCommunities.add(c.cid):hiddenCommunities.delete(c.cid));updateVisibility()}}const legendEl=document.getElementById('legend');LEGEND.forEach(c=>{{const el=document.createElement('div');el.className='legend-item';el.innerHTML=`<input type="checkbox" checked><div class="legend-dot" style="background:${{c.color}}"></div><span>${{c.label}}</span><span>${{c.count}}</span>`;el.querySelector('input').onchange=e=>{{e.target.checked?hiddenCommunities.delete(c.cid):hiddenCommunities.add(c.cid);el.classList.toggle('dimmed',!e.target.checked);updateVisibility()}};legendEl.appendChild(el)}});
</script><script>const hyperedges={hyperedges};network.on('afterDrawing',ctx=>{{hyperedges.forEach(h=>{{const p=h.nodes.map(id=>network.getPositions([id])[id]).filter(Boolean);if(p.length<2)return;const cx=p.reduce((s,x)=>s+x.x,0)/p.length,cy=p.reduce((s,x)=>s+x.y,0)/p.length;ctx.save();ctx.globalAlpha=.12;ctx.fillStyle='#6366f1';ctx.beginPath();ctx.moveTo(p[0].x,p[0].y);p.slice(1).forEach(x=>ctx.lineTo(x.x,x.y));ctx.closePath();ctx.fill();ctx.restore()}})}});</script></body></html>"##
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
    fn explicit_limit_builds_community_meta_graph() -> Result<(), Box<dyn Error>> {
        let graph: GraphDocument = serde_json::from_value(json!({
            "nodes":[
                {"id":"a","label":"A"},{"id":"b","label":"B"},
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
        Ok(())
    }

    #[test]
    fn sidecar_overlay_is_loaded_and_staleness_recomputed() -> Result<(), Box<dyn Error>> {
        let directory = tempdir()?;
        let out = directory.path().join("graphify-out");
        fs::create_dir(&out)?;
        fs::write(directory.path().join("source.rs"), "fn main() {}")?;
        let digest = format!(
            "{:x}",
            Sha256::digest(fs::read(directory.path().join("source.rs"))?)
        );
        fs::write(
            out.join(".graphify_learning.json"),
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
}
