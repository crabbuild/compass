use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha1::{Digest, Sha1};
use trail_files::write_text_atomic;
use trail_graph::Communities;
use trail_model::{EdgeRecord, GraphDocument, NodeRecord};

use crate::OutputError;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallflowSection {
    pub id: String,
    pub name: String,
    pub communities: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct CallflowOptions<'a> {
    pub community_labels: Option<&'a BTreeMap<usize, String>>,
    pub sections: Option<&'a [CallflowSection]>,
    pub report: &'a str,
    pub project_name: &'a str,
    pub built_at_commit: Option<&'a str>,
    pub language: &'a str,
    pub max_sections: usize,
    pub diagram_scale: f64,
    pub max_diagram_nodes: usize,
    pub max_diagram_edges: usize,
    pub generated_at: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallflowExport {
    pub loaded_sections: usize,
    pub rendered_sections: usize,
    pub mermaid_diagrams: usize,
    pub call_tables: usize,
}

impl Default for CallflowOptions<'_> {
    fn default() -> Self {
        Self {
            community_labels: None,
            sections: None,
            report: "",
            project_name: "Project",
            built_at_commit: None,
            language: "auto",
            max_sections: 15,
            diagram_scale: 1.0,
            max_diagram_nodes: 18,
            max_diagram_edges: 24,
            generated_at: None,
        }
    }
}

const ARCHETYPES: [(&str, &str, &str, &[&str]); 9] = [
    (
        "extract-pipeline",
        "提取管线",
        "Extraction Pipeline",
        &[
            "extract",
            "extractor",
            "tree",
            "sitter",
            "parser",
            "language",
            "python",
            "javascript",
            "typescript",
            "rust",
            "java",
            "go",
            "ast",
            "calls",
            "imports",
            "multilang",
        ],
    ),
    (
        "build-graph",
        "图谱构建",
        "Graph Build",
        &[
            "build",
            "graph",
            "merge",
            "dedup",
            "node",
            "edge",
            "hyperedge",
            "json",
            "schema",
            "normalize",
            "confidence",
        ],
    ),
    (
        "analysis-clustering",
        "分析聚类",
        "Analysis & Clustering",
        &[
            "cluster",
            "community",
            "leiden",
            "cohesion",
            "analyze",
            "god",
            "surprise",
            "question",
            "query",
            "path",
            "explain",
            "benchmark",
        ],
    ),
    (
        "outputs-docs",
        "输出文档",
        "Outputs & Docs",
        &[
            "export",
            "html",
            "wiki",
            "obsidian",
            "canvas",
            "svg",
            "graphml",
            "report",
            "callflow",
            "mermaid",
            "tree",
            "documentation",
        ],
    ),
    (
        "cli-skills",
        "CLI 与技能安装",
        "CLI & Skill Installers",
        &[
            "main",
            "install",
            "uninstall",
            "skill",
            "agent",
            "claude",
            "codex",
            "opencode",
            "aider",
            "copilot",
            "kiro",
            "vscode",
            "hook",
            "command",
        ],
    ),
    (
        "ingest-cache-update",
        "摄取与增量更新",
        "Ingestion & Updates",
        &[
            "ingest",
            "fetch",
            "download",
            "url",
            "html",
            "markdown",
            "cache",
            "manifest",
            "watch",
            "update",
            "incremental",
            "transcribe",
            "video",
            "audio",
            "google",
        ],
    ),
    (
        "serve-api",
        "服务 API",
        "Serving API",
        &[
            "serve", "api", "request", "response", "endpoint", "router", "handle", "upload",
            "search", "delete", "enrich",
        ],
    ),
    (
        "security-global",
        "安全与全局图",
        "Security & Global Graph",
        &[
            "security",
            "safe",
            "ssrf",
            "xss",
            "path",
            "traversal",
            "global",
            "prefix",
            "prune",
            "repo",
            "clone",
        ],
    ),
    (
        "tests-fixtures",
        "测试与样例",
        "Tests & Fixtures",
        &[
            "test", "tests", "fixture", "fixtures", "sample", "assert", "pytest", "mock",
        ],
    ),
];

#[must_use]
pub fn derive_callflow_sections(
    document: &GraphDocument,
    communities: &Communities,
    labels: Option<&BTreeMap<usize, String>>,
    language: &str,
    max_sections: usize,
) -> Vec<CallflowSection> {
    let language = detect_language(language, document, labels);
    let max_sections = if max_sections == 0 { 15 } else { max_sections };
    let mut sections = vec![CallflowSection {
        id: "overview".into(),
        name: text(language, "架构总览", "Architecture Overview").into(),
        communities: Vec::new(),
    }];
    let indexed = indexed_communities(document, communities);
    let mut ordered = indexed.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| {
        right
            .1
            .len()
            .cmp(&left.1.len())
            .then_with(|| left.0.to_string().cmp(&right.0.to_string()))
    });
    #[derive(Clone)]
    struct Group {
        priority: usize,
        id: &'static str,
        name: &'static str,
        communities: Vec<String>,
        nodes: usize,
    }
    let mut grouped = Vec::<Group>::new();
    let mut unassigned = Vec::<(String, String)>::new();
    for (community, nodes) in ordered {
        let label = labels
            .and_then(|labels| labels.get(community).cloned())
            .unwrap_or_else(|| fallback_community_label(*community, nodes, language));
        let mut body = label.clone();
        for node in nodes.iter().take(80) {
            body.push(' ');
            body.push_str(&node.string("label"));
            body.push(' ');
            body.push_str(&node.string("source_file"));
            body.push(' ');
            body.push_str(&node.string("node_type"));
            body.push(' ');
            body.push_str(&node.string("file_type"));
        }
        let body = body.to_lowercase();
        let mut best = None;
        let mut best_score = 0;
        for (priority, (id, zh, en, keywords)) in ARCHETYPES.iter().enumerate() {
            let score = keywords
                .iter()
                .map(|keyword| keyword_count(&body, keyword))
                .sum();
            if score > best_score {
                best = Some((priority, *id, text(language, zh, en)));
                best_score = score;
            }
        }
        if let Some((priority, id, name)) = best.filter(|_| best_score >= 2) {
            if let Some(group) = grouped.iter_mut().find(|group| group.id == id) {
                group.communities.push(community.to_string());
                group.nodes += nodes.len();
            } else {
                grouped.push(Group {
                    priority,
                    id,
                    name,
                    communities: vec![community.to_string()],
                    nodes: nodes.len(),
                });
            }
        } else {
            unassigned.push((community.to_string(), label));
        }
    }
    grouped.sort_by(|left, right| {
        left.priority
            .cmp(&right.priority)
            .then_with(|| right.nodes.cmp(&left.nodes))
            .then_with(|| left.id.cmp(right.id))
    });
    let grouped_cap = max_sections.max(1).saturating_sub(1).max(1);
    let overflow = grouped
        .iter()
        .skip(grouped_cap)
        .flat_map(|group| group.communities.iter().cloned())
        .collect::<Vec<_>>();
    sections.extend(
        grouped
            .into_iter()
            .take(grouped_cap)
            .map(|group| CallflowSection {
                id: group.id.into(),
                name: group.name.into(),
                communities: group.communities,
            }),
    );
    let remaining = max_sections
        .saturating_sub(sections.len().saturating_sub(1))
        .saturating_sub(1);
    sections.extend(
        unassigned
            .iter()
            .take(remaining)
            .map(|(community, label)| CallflowSection {
                id: label.clone(),
                name: label.clone(),
                communities: vec![community.clone()],
            }),
    );
    let others = overflow
        .into_iter()
        .chain(unassigned.into_iter().skip(remaining).map(|item| item.0))
        .collect::<Vec<_>>();
    if !others.is_empty() {
        sections.push(CallflowSection {
            id: "other".into(),
            name: text(language, "其他", "Other").into(),
            communities: others,
        });
    }
    sections
}

pub fn callflow_html_document(
    document: &GraphDocument,
    communities: &Communities,
    options: &CallflowOptions<'_>,
) -> Result<String, OutputError> {
    if document.nodes.is_empty() {
        return Err(OutputError::EmptyCallflowGraph);
    }
    let language = detect_language(options.language, document, options.community_labels);
    let raw_sections = options.sections.map_or_else(
        || {
            derive_callflow_sections(
                document,
                communities,
                options.community_labels,
                language,
                options.max_sections,
            )
        },
        <[CallflowSection]>::to_vec,
    );
    let sections = normalize_sections(&raw_sections, language);
    if sections.len() <= 1 {
        return Err(OutputError::NoCallflowSections);
    }
    let section_nodes = section_nodes(document, communities, &sections);
    let node_section = section_nodes
        .iter()
        .flat_map(|(section, nodes)| {
            nodes
                .iter()
                .map(move |node| (node.id.as_str(), section.as_str()))
        })
        .collect::<HashMap<_, _>>();
    let project = if options.project_name.is_empty() {
        "Project"
    } else {
        options.project_name
    };
    let commit = options
        .built_at_commit
        .unwrap_or("unknown")
        .chars()
        .take(7)
        .collect::<String>();
    let title = if is_zh(language) {
        format!("{project} — 完整调用流程与架构文档")
    } else {
        format!("{project} — Complete Call Flow & Architecture Documentation")
    };
    let subtitle = if is_zh(language) {
        format!(
            "由 graphify 知识图谱生成：{} 个节点、{} 条边、{} 个社区。Commit: {commit}",
            document.nodes.len(),
            document.links.len(),
            indexed_communities(document, communities).len()
        )
    } else {
        format!(
            "Generated from graphify knowledge graph: {} nodes, {} edges, {} communities. Commit: {commit}",
            document.nodes.len(),
            document.links.len(),
            indexed_communities(document, communities).len()
        )
    };
    let mut html = format!(
        r#"<!DOCTYPE html><html lang="{}"><head><meta charset="UTF-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>{}</title><script src="https://cdn.jsdelivr.net/npm/mermaid@11/dist/mermaid.min.js"></script><style>{}</style></head><body><div class="container"><h1>{}</h1><p class="subtitle">{}</p>{}"#,
        html_escape(language, true),
        html_escape(&title, false),
        CSS,
        html_escape(&title, false),
        html_escape(&subtitle, false),
        navigation(&sections)
    );
    html.push_str(&format!("<!-- ====== Architecture Overview ====== --><h2 id=\"overview\">1. {}</h2><div class=\"mermaid\">{}</div>", html_escape(&sections[0].name, false), overview_mermaid(&sections, &section_nodes, document, &node_section, language, options.diagram_scale)));
    html.push_str(&overview_cards(
        &sections,
        &section_nodes,
        document,
        &node_section,
        language,
    ));
    if let Some(card) = report_highlights(options.report, language) {
        html.push_str(&format!("<div class=\"grid\">{card}</div>"));
    }
    html.push_str("<hr>");
    for (index, section) in sections
        .iter()
        .filter(|section| section.id != "overview")
        .enumerate()
    {
        let nodes = section_nodes.get(&section.id).cloned().unwrap_or_default();
        let edges = document
            .links
            .iter()
            .filter(|edge| {
                node_section.get(edge_source(edge)) == Some(&section.id.as_str())
                    && node_section.get(edge_target(edge)) == Some(&section.id.as_str())
            })
            .collect::<Vec<_>>();
        html.push_str(&format!("<!-- ====== {}. {} ====== --><h2 id=\"{}\">{}. {}</h2><p>{}</p><div class=\"mermaid\">{}</div>", index + 2, comment_text(&section.name), html_escape(&section.id, true), index + 2, html_escape(&section.name, false), section_intro(section, &nodes, edges.len(), language), section_mermaid(section, &nodes, &edges, language, options)));
        html.push_str(&call_table(&nodes, &edges, language));
        html.push_str(&section_cards(&nodes, &edges, language));
        html.push_str("<hr>");
    }
    append_hyperedges(&mut html, document);
    let indexed = indexed_communities(document, communities);
    let extracted = document
        .links
        .iter()
        .filter(|edge| defaulted(edge, "confidence", "EXTRACTED") == "EXTRACTED")
        .count();
    let inferred = document
        .links
        .iter()
        .filter(|edge| defaulted(edge, "confidence", "EXTRACTED") == "INFERRED")
        .count();
    let ambiguous = document
        .links
        .iter()
        .filter(|edge| defaulted(edge, "confidence", "EXTRACTED") == "AMBIGUOUS")
        .count();
    html.push_str(&format!("<h2 id=\"stats\">Project Statistics</h2><div class=\"grid\"><div class=\"card\"><h4>Graph</h4><table><tr><td>Nodes</td><td>{}</td></tr><tr><td>Edges</td><td>{}</td></tr><tr><td>Hyperedges</td><td>{}</td></tr><tr><td>Communities</td><td>{}</td></tr><tr><td>Documented Sections</td><td>{}</td></tr></table></div><div class=\"card\"><h4>Edge Confidence</h4><table><tr><td>EXTRACTED</td><td>{extracted}</td></tr><tr><td>INFERRED</td><td>{inferred}</td></tr><tr><td>AMBIGUOUS</td><td>{ambiguous}</td></tr></table></div></div>", document.nodes.len(), document.links.len(), hyperedges(document).len(), indexed.len(), sections.len()-1));
    let generated = options
        .generated_at
        .map(ToOwned::to_owned)
        .unwrap_or_else(utc_minute);
    html.push_str(&format!("<footer><p>{} — Architecture Documentation</p><p>Generated: {} · graphify callflow-html</p></footer></div><script>mermaid.initialize({{startOnLoad:true,theme:'dark',securityLevel:'loose',flowchart:{{htmlLabels:true,useMaxWidth:true}}}});</script></body></html>", html_escape(project, false), html_escape(&generated, false)));
    Ok(html)
}

pub fn write_callflow_html(
    document: &GraphDocument,
    communities: &Communities,
    output_path: impl AsRef<Path>,
    options: &CallflowOptions<'_>,
) -> Result<CallflowExport, OutputError> {
    let language = detect_language(options.language, document, options.community_labels);
    let raw_sections = options.sections.map_or_else(
        || {
            derive_callflow_sections(
                document,
                communities,
                options.community_labels,
                language,
                options.max_sections,
            )
        },
        <[CallflowSection]>::to_vec,
    );
    let loaded_sections = normalize_sections(&raw_sections, language).len();
    let html = callflow_html_document(document, communities, options)?;
    let result = CallflowExport {
        loaded_sections,
        rendered_sections: html.matches("<h2 id=").count(),
        mermaid_diagrams: html.matches("<div class=\"mermaid\">").count(),
        call_tables: html.matches("<table class=\"call-table\">").count(),
    };
    write_text_atomic(output_path, &html)?;
    Ok(result)
}

fn normalize_sections(sections: &[CallflowSection], language: &str) -> Vec<CallflowSection> {
    let mut output = vec![CallflowSection {
        id: "overview".into(),
        name: text(language, "架构总览", "Architecture Overview").into(),
        communities: Vec::new(),
    }];
    let mut used = HashSet::from([
        "overview".to_owned(),
        "hyperedges".to_owned(),
        "stats".to_owned(),
    ]);
    for section in sections {
        if section.id.to_lowercase() == "overview" {
            output[0].name.clone_from(&section.name);
            continue;
        }
        let id = anchor_id(&section.id, &mut used);
        output.push(CallflowSection {
            id,
            name: section.name.clone(),
            communities: section.communities.clone(),
        });
    }
    output
}

fn indexed_communities<'a>(
    document: &'a GraphDocument,
    communities: &Communities,
) -> BTreeMap<usize, Vec<&'a NodeRecord>> {
    let lookup = communities
        .iter()
        .flat_map(|(community, members)| {
            members
                .iter()
                .map(move |member| (member.as_str(), *community))
        })
        .collect::<HashMap<_, _>>();
    let mut output = BTreeMap::<usize, Vec<&NodeRecord>>::new();
    for node in &document.nodes {
        let community = lookup
            .get(node.id.as_str())
            .copied()
            .or_else(|| {
                node.attributes
                    .get("community")
                    .and_then(Value::as_u64)
                    .map(|value| value as usize)
            })
            .unwrap_or(0);
        output.entry(community).or_default().push(node);
    }
    output
}

fn section_nodes<'a>(
    document: &'a GraphDocument,
    communities: &Communities,
    sections: &[CallflowSection],
) -> HashMap<String, Vec<&'a NodeRecord>> {
    let indexed = indexed_communities(document, communities);
    sections
        .iter()
        .map(|section| {
            let nodes = section
                .communities
                .iter()
                .flat_map(|community| {
                    community
                        .parse::<usize>()
                        .ok()
                        .and_then(|community| indexed.get(&community))
                        .into_iter()
                        .flatten()
                        .copied()
                })
                .collect();
            (section.id.clone(), nodes)
        })
        .collect()
}

fn overview_mermaid(
    sections: &[CallflowSection],
    nodes: &HashMap<String, Vec<&NodeRecord>>,
    document: &GraphDocument,
    node_section: &HashMap<&str, &str>,
    language: &str,
    scale: f64,
) -> String {
    let mut lines = vec![mermaid_init(scale)];
    for section in sections.iter().filter(|section| section.id != "overview") {
        let id = stable_id(&section.id, "section").to_uppercase();
        lines.push(format!(
            "    {id}(\"{}<br/><small>{} nodes</small>\")",
            mermaid_text(&section.name),
            nodes.get(&section.id).map(Vec::len).unwrap_or_default()
        ));
    }
    let mut counts = Vec::<((String, String), usize, BTreeMap<String, usize>)>::new();
    let mut positions = HashMap::new();
    for edge in &document.links {
        if !include_edge(edge) {
            continue;
        }
        let (Some(source), Some(target)) = (
            node_section.get(edge_source(edge)),
            node_section.get(edge_target(edge)),
        ) else {
            continue;
        };
        if source == target {
            continue;
        }
        let key = ((*source).to_owned(), (*target).to_owned());
        let position = *positions.entry(key.clone()).or_insert_with(|| {
            counts.push((key, 0, BTreeMap::new()));
            counts.len() - 1
        });
        counts[position].1 += 1;
        *counts[position]
            .2
            .entry(edge.string("relation"))
            .or_default() += 1;
    }
    counts.sort_by_key(|item| std::cmp::Reverse(item.1));
    for ((source, target), count, relations) in counts.into_iter().take(12) {
        let relation = relations
            .into_iter()
            .max_by_key(|(_, count)| *count)
            .map(|item| item.0)
            .unwrap_or_else(|| "relates".into());
        let label = if count > 1 {
            format!("{} x{count}", relation_text(&relation, language))
        } else {
            relation_text(&relation, language)
        };
        lines.push(format!(
            "    {} -->|{label}| {}",
            stable_id(&source, "section").to_uppercase(),
            stable_id(&target, "section").to_uppercase()
        ));
    }
    lines.join("\n")
}

fn section_mermaid(
    section: &CallflowSection,
    nodes: &[&NodeRecord],
    edges: &[&EdgeRecord],
    language: &str,
    options: &CallflowOptions<'_>,
) -> String {
    let mut lines = vec![
        mermaid_init(options.diagram_scale),
        format!(
            "    %% Section: {} ({} nodes, {} edges)",
            mermaid_text(&section.name),
            nodes.len(),
            edges.len()
        ),
    ];
    let selected = nodes
        .iter()
        .take(options.max_diagram_nodes)
        .copied()
        .collect::<Vec<_>>();
    let ids = selected
        .iter()
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    for node in &selected {
        lines.push(format!(
            "    {}(\"{}\")",
            stable_id(&node.id, "node"),
            node_mermaid_label(node)
        ));
    }
    for edge in edges
        .iter()
        .filter(|edge| {
            include_edge(edge) && ids.contains(edge_source(edge)) && ids.contains(edge_target(edge))
        })
        .take(options.max_diagram_edges)
    {
        lines.push(format!(
            "    {} -->|{}| {}",
            stable_id(edge_source(edge), "node"),
            relation_text(&edge.string("relation"), language),
            stable_id(edge_target(edge), "node")
        ));
    }
    lines.join("\n")
}

fn call_table(nodes: &[&NodeRecord], edges: &[&EdgeRecord], language: &str) -> String {
    let mut callers = HashMap::<&str, Vec<&str>>::new();
    let mut callees = HashMap::<&str, Vec<&str>>::new();
    for edge in edges {
        if matches!(
            edge.string("relation").as_str(),
            "calls" | "imports" | "imports_from" | "uses" | "method"
        ) {
            callers
                .entry(edge_target(edge))
                .or_default()
                .push(edge_source(edge));
            callees
                .entry(edge_source(edge))
                .or_default()
                .push(edge_target(edge));
        }
    }
    let lookup = nodes
        .iter()
        .map(|node| (node.id.as_str(), *node))
        .collect::<HashMap<_, _>>();
    let mut body = String::new();
    for (index, node) in nodes.iter().take(30).enumerate() {
        let label = node.label();
        let path = short_path(&node.string("source_file"));
        body.push_str(&format!("<tr><td>{}</td><td><code>{}</code><br><small>{}</small></td><td><span class=\"tag tag-func\">{}</span></td><td>{}</td><td>{}</td><td>{}</td></tr>", index+1, html_escape(label,false), html_escape(&path,false), text(language,"函数","Function"), refs(callers.get(node.id.as_str()), &lookup, language, true), refs(callees.get(node.id.as_str()), &lookup, language, false), html_escape(&format!("{} node in {}.", label, if path.is_empty(){"project"}else{&path}),false)));
    }
    format!(
        "<h3>{}</h3><table class=\"call-table\"><tr><th>#</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th></tr>{body}</table>",
        text(language, "调用明细", "Call Details"),
        text(language, "节点", "Node"),
        text(language, "类型", "Type"),
        text(language, "调用方", "Caller"),
        text(language, "被调用/依赖", "Callees"),
        text(language, "说明", "Description")
    )
}

fn refs(
    ids: Option<&Vec<&str>>,
    lookup: &HashMap<&str, &NodeRecord>,
    language: &str,
    inbound: bool,
) -> String {
    let Some(ids) = ids.filter(|ids| !ids.is_empty()) else {
        return text(
            language,
            if inbound {
                "外部入口 / 无直接入边"
            } else {
                "无直接出边"
            },
            if inbound {
                "External entry / no inbound edge"
            } else {
                "No direct outbound edge"
            },
        )
        .into();
    };
    ids.iter()
        .take(3)
        .map(|id| {
            format!(
                "<code>{}</code>",
                html_escape(lookup.get(id).map_or(*id, |node| node.label()), false)
            )
        })
        .collect::<Vec<_>>()
        .join("<br>")
}

fn section_intro(
    section: &CallflowSection,
    nodes: &[&NodeRecord],
    edges: usize,
    language: &str,
) -> String {
    let files = nodes
        .iter()
        .map(|node| short_path(&node.string("source_file")))
        .filter(|path| !path.is_empty())
        .take(3)
        .collect::<Vec<_>>()
        .join(", ");
    let value = if is_zh(language) {
        format!(
            "{} 汇集了相关实现，主要分布在 {}。本节覆盖 {} 个节点、{edges} 条内部边，图中只展示最有代表性的调用关系以保持可读性。",
            section.name,
            if files.is_empty() {
                "未标注源文件"
            } else {
                &files
            },
            nodes.len()
        )
    } else {
        format!(
            "{} groups related implementation, mostly in {}. This section covers {} nodes and {edges} internal edges; the diagram shows representative relationships to stay readable.",
            section.name,
            if files.is_empty() {
                "unmapped files"
            } else {
                &files
            },
            nodes.len()
        )
    };
    html_escape(&value, false)
}

fn section_cards(nodes: &[&NodeRecord], edges: &[&EdgeRecord], language: &str) -> String {
    let mut files = BTreeMap::<String, usize>::new();
    for node in nodes {
        let path = node.string("source_file");
        if !path.is_empty() {
            *files.entry(path).or_default() += 1;
        }
    }
    let rows = files
        .into_iter()
        .take(8)
        .map(|(path, count)| {
            format!(
                "<tr><td><code>{}</code></td><td>{count} {}</td></tr>",
                html_escape(&short_path(&path), false),
                text(language, "个节点", "nodes")
            )
        })
        .collect::<String>();
    let relations = edges
        .iter()
        .filter(|edge| include_edge(edge))
        .map(|edge| edge.string("relation"))
        .take(4)
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "<div class=\"grid\"><div class=\"card\"><h4>{}</h4><table>{rows}</table></div><div class=\"card\"><h4>{}</h4><p>{}</p></div></div>",
        text(language, "关键文件", "Key Files"),
        text(language, "设计备注", "Design Notes"),
        html_escape(&relations, false)
    )
}

fn overview_cards(
    sections: &[CallflowSection],
    nodes: &HashMap<String, Vec<&NodeRecord>>,
    document: &GraphDocument,
    node_section: &HashMap<&str, &str>,
    language: &str,
) -> String {
    let rows = sections
        .iter()
        .filter(|s| s.id != "overview")
        .map(|s| {
            format!(
                "<tr><td>{}</td><td>{}</td><td><code>{}</code></td></tr>",
                html_escape(&s.name, false),
                nodes.get(&s.id).map(Vec::len).unwrap_or_default(),
                s.communities.join(", ")
            )
        })
        .collect::<String>();
    let mut flow = sections
        .iter()
        .filter(|s| s.id != "overview")
        .map(|s| s.name.clone())
        .collect::<Vec<_>>();
    flow.truncate(7);
    let _ = (document, node_section);
    format!(
        "<div class=\"grid\"><div class=\"card\"><h4>{}</h4><table>{rows}</table></div><div class=\"card\"><h4>{}</h4><div class=\"arrow-chain\">{}</div></div></div>",
        text(language, "架构层次", "Architecture Layers"),
        text(language, "核心数据流", "Core Flow"),
        html_escape(&flow.join(" -> "), false)
    )
}

fn report_highlights(report: &str, language: &str) -> Option<String> {
    let mut keep = Vec::new();
    let (mut summary, mut gods) = (false, false);
    for line in report.lines() {
        let line = line.trim();
        if line.starts_with("## ") {
            summary = line == "## Summary";
            gods = line.starts_with("## God Nodes");
            continue;
        }
        if summary && line.starts_with("- ") {
            keep.push(&line[2..]);
        } else if gods
            && line.chars().next().is_some_and(|c| c.is_ascii_digit())
            && line.contains('.')
        {
            keep.push(line);
        }
        if keep.len() >= 6 {
            break;
        }
    }
    if keep.is_empty() {
        None
    } else {
        Some(format!(
            "<div class=\"card\"><h4>{}</h4><ul>{}</ul></div>",
            text(language, "图谱报告摘要", "Graph Report Highlights"),
            keep.into_iter()
                .map(|v| format!("<li>{}</li>", html_escape(v, false)))
                .collect::<String>()
        ))
    }
}

fn append_hyperedges(html: &mut String, document: &GraphDocument) {
    let values = hyperedges(document);
    if values.is_empty() {
        return;
    }
    html.push_str(
        "<h2 id=\"hyperedges\">Group Relationships (Hyperedges)</h2><div class=\"grid\">",
    );
    for value in values.iter().take(9) {
        let Some(item) = value.as_object() else {
            continue;
        };
        let label = item
            .get("label")
            .or_else(|| item.get("id"))
            .and_then(Value::as_str)
            .unwrap_or("?");
        let relation = item
            .get("relation")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let nodes = item
            .get("nodes")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        html.push_str(&format!(
            "<div class=\"card\"><h4>{}</h4><p><code>{}</code> — {} participants</p><ul>",
            html_escape(label, false),
            html_escape(relation, false),
            nodes.len()
        ));
        for node in nodes.iter().take(5) {
            html.push_str(&format!(
                "<li><code>{}</code></li>",
                html_escape(node.as_str().unwrap_or_default(), false)
            ));
        }
        html.push_str("</ul></div>");
    }
    html.push_str("</div><hr>");
}
fn hyperedges(document: &GraphDocument) -> Vec<Value> {
    document
        .extras
        .get("hyperedges")
        .or_else(|| document.graph.get("hyperedges"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn navigation(sections: &[CallflowSection]) -> String {
    format!(
        "<div class=\"nav\">{}</div>",
        sections
            .iter()
            .map(|s| format!(
                "<a href=\"#{}\">{}</a>",
                html_escape(&s.id, true),
                html_escape(&s.name, false)
            ))
            .collect::<String>()
    )
}
fn fallback_community_label(community: usize, nodes: &[&NodeRecord], language: &str) -> String {
    let words = keywords(nodes, 3);
    if words.is_empty() {
        if is_zh(language) {
            format!("社区 {community}")
        } else {
            format!("Community {community}")
        }
    } else {
        words
            .into_iter()
            .map(|word| {
                let mut chars = word.chars();
                chars.next().map_or(String::new(), |first| {
                    first.to_uppercase().collect::<String>() + chars.as_str()
                })
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}
fn keywords(nodes: &[&NodeRecord], limit: usize) -> Vec<String> {
    const STOPWORDS: [&str; 27] = [
        "the", "and", "for", "with", "from", "this", "that", "class", "function", "method", "file",
        "src", "lib", "core", "index", "main", "init", "py", "ts", "tsx", "js", "jsx", "go", "rs",
        "java", "html", "css",
    ];
    let mut counts = HashMap::<String, (usize, usize)>::new();
    let mut order = 0;
    for node in nodes {
        for raw in format!("{} {}", node.string("label"), node.string("source_file"))
            .replace(['/', '_', '-'], " ")
            .split_whitespace()
        {
            let word = raw
                .chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<String>()
                .to_lowercase();
            if word.len() < 3 || STOPWORDS.contains(&word.as_str()) {
                continue;
            }
            let entry = counts.entry(word).or_insert_with(|| {
                let x = (0, order);
                order += 1;
                x
            });
            entry.0 += 1;
        }
    }
    let mut values = counts.into_iter().collect::<Vec<_>>();
    values.sort_by_key(|(_, v)| (std::cmp::Reverse(v.0), v.1));
    values.into_iter().take(limit).map(|v| v.0).collect()
}
fn keyword_count(body: &str, word: &str) -> usize {
    body.match_indices(word)
        .filter(|(index, _)| {
            let before = body.as_bytes().get(index.wrapping_sub(1)).copied();
            let after = body.as_bytes().get(index + word.len()).copied();
            !before.is_some_and(|c| c.is_ascii_alphanumeric())
                && !after.is_some_and(|c| c.is_ascii_alphanumeric())
        })
        .count()
}
fn anchor_id(raw: &str, used: &mut HashSet<String>) -> String {
    let mut base = raw
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() {
                c
            } else {
                '-'
            }
        })
        .collect::<String>();
    while base.contains("--") {
        base = base.replace("--", "-");
    }
    base = base.trim_matches('-').chars().take(48).collect();
    if base.is_empty() {
        base = "section".into();
    }
    let mut candidate = base.clone();
    if used.contains(&candidate) {
        candidate = format!(
            "{base}-{}",
            &format!("{:x}", Sha1::digest(raw.as_bytes()))[..6]
        );
    }
    let mut suffix = 2;
    while used.contains(&candidate) {
        candidate = format!("{base}-{suffix}");
        suffix += 1;
    }
    used.insert(candidate.clone());
    candidate
}
fn stable_id(raw: &str, prefix: &str) -> String {
    let mut slug = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    while slug.contains("__") {
        slug = slug.replace("__", "_");
    }
    slug = slug.trim_matches('_').chars().take(48).collect();
    if slug.is_empty() {
        slug = prefix.into();
    }
    if slug.starts_with(|c: char| c.is_ascii_digit()) {
        slug = format!("{prefix}_{slug}");
    }
    format!(
        "{}_{}",
        slug.trim_end_matches('_'),
        &format!("{:x}", Sha1::digest(raw.as_bytes()))[..8]
    )
}
fn mermaid_init(scale: f64) -> String {
    let scale = scale.clamp(0.65, 1.8);
    let config = serde_json::json!({
        "theme": "dark",
        "themeVariables": {"fontSize": format!("{:.1}px", 15.0 * scale)},
        "flowchart": {
            "htmlLabels": true,
            "nodeSpacing": (48.0 * scale).round() as i64,
            "rankSpacing": (64.0 * scale).round() as i64,
        }
    });
    format!("%%{{init: {config}}}%%\nflowchart LR")
}
fn node_mermaid_label(node: &NodeRecord) -> String {
    let label = humanize(node.label(), &node.string("source_file"));
    let path = short_path(&node.string("source_file"));
    if !path.is_empty()
        && !label.ends_with(
            Path::new(&path)
                .file_name()
                .and_then(|v| v.to_str())
                .unwrap_or_default(),
        )
    {
        format!(
            "{}<br/><small>{}</small>",
            mermaid_text(&label),
            mermaid_text(&path)
        )
    } else {
        mermaid_text(&label)
    }
}
fn humanize(label: &str, path: &str) -> String {
    let mut value: String = if label.is_empty() {
        Path::new(path)
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("Unknown")
            .into()
    } else {
        label.trim().into()
    };
    if value.starts_with('.') && value.ends_with("()") {
        value.remove(0);
    }
    if value.len() > 42 {
        value.truncate(value.char_indices().nth(39).map_or(value.len(), |v| v.0));
        value.push_str("...");
    }
    value
}
fn short_path(path: &str) -> String {
    let parts = path.split('/').collect::<Vec<_>>();
    if parts.len() > 3 {
        parts[parts.len() - 3..].join("/")
    } else {
        path.into()
    }
}
fn relation_text(relation: &str, language: &str) -> String {
    let value = if is_zh(language) {
        match relation {
            "calls" => "调用",
            "uses" => "使用",
            "imports" | "imports_from" => "导入",
            "method" => "方法",
            "contains" => "包含",
            _ => relation,
        }
    } else {
        match relation {
            "imports_from" => "imports",
            "rationale_for" => "explains",
            "conceptually_related_to" => "relates",
            _ => relation,
        }
    };
    mermaid_text(&value.replace('_', " "))
}
fn mermaid_text(value: &str) -> String {
    html_escape(
        &value
            .replace('"', "'")
            .replace(['`', '#', '{', '}'], "")
            .replace('|', " ")
            .replace("->>", " to ")
            .replace("-->", " to ")
            .replace("->", " to ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
        false,
    )
}
fn include_edge(edge: &EdgeRecord) -> bool {
    let confidence = defaulted(edge, "confidence", "EXTRACTED");
    confidence == "EXTRACTED"
        || (confidence == "INFERRED"
            && edge
                .attributes
                .get("confidence_score")
                .and_then(Value::as_f64)
                .unwrap_or(1.0)
                >= 0.85)
}
fn defaulted(edge: &EdgeRecord, key: &str, default: &str) -> String {
    let value = edge.string(key);
    if value.is_empty() {
        default.into()
    } else {
        value
    }
}
fn edge_source(edge: &EdgeRecord) -> &str {
    edge.attributes
        .get("_src")
        .and_then(Value::as_str)
        .unwrap_or(&edge.source)
}
fn edge_target(edge: &EdgeRecord) -> &str {
    edge.attributes
        .get("_tgt")
        .and_then(Value::as_str)
        .unwrap_or(&edge.target)
}
fn detect_language<'a>(
    language: &'a str,
    document: &GraphDocument,
    labels: Option<&BTreeMap<usize, String>>,
) -> &'a str {
    if !language.eq_ignore_ascii_case("auto") {
        return language;
    }
    let chinese = labels
        .into_iter()
        .flat_map(|v| v.values())
        .any(|v| v.chars().any(is_chinese))
        || document
            .nodes
            .iter()
            .take(200)
            .any(|n| n.label().chars().any(is_chinese));
    if chinese { "zh-CN" } else { "en" }
}
fn is_chinese(c: char) -> bool {
    ('\u{4e00}'..='\u{9fff}').contains(&c)
}
fn is_zh(language: &str) -> bool {
    language.to_lowercase().starts_with("zh")
}
fn text<'a>(language: &str, zh: &'a str, en: &'a str) -> &'a str {
    if is_zh(language) { zh } else { en }
}
fn html_escape(value: &str, quote: bool) -> String {
    let mut output = value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    if quote {
        output = output.replace('"', "&quot;").replace('\'', "&#x27;");
    }
    output
}
fn comment_text(value: &str) -> String {
    value.replace("--", "- -").replace(['\n', '\r'], " ")
}
fn utc_minute() -> String {
    let now = time::OffsetDateTime::now_utc();
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02} UTC",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute()
    )
}

const CSS: &str = r#":root{--bg:#0f172a;--surface:#1e293b;--border:#334155;--text:#e2e8f0;--muted:#94a3b8;--accent:#38bdf8}*{box-sizing:border-box}body{margin:0;font-family:'Segoe UI',system-ui,sans-serif;background:var(--bg);color:var(--text);line-height:1.7}.container{max-width:1200px;margin:auto;padding:40px 24px}h1{font-size:2.4rem;color:var(--accent)}h2{margin:48px 0 16px;border-bottom:2px solid var(--accent)}h3,h4{color:var(--accent)}p,small{color:var(--muted)}.nav{position:sticky;top:0;background:var(--bg);display:flex;gap:20px;flex-wrap:wrap;padding:12px 0}.nav a{color:var(--accent)}.mermaid,.card{background:var(--surface);border:1px solid var(--border);border-radius:12px;padding:20px;margin:16px 0;overflow:auto}.grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(340px,1fr));gap:16px}.call-table,table{width:100%;border-collapse:collapse}.call-table th,.call-table td,td,th{padding:8px;border:1px solid var(--border)}code{background:#ffffff10;padding:1px 6px}.tag{display:inline-block;padding:2px 8px;border-radius:4px}.tag-func{color:var(--accent)}.arrow-chain{font-family:monospace;color:var(--accent)}footer{text-align:center;padding:40px 0}"#;

#[cfg(test)]
mod tests {
    use std::error::Error;

    use serde_json::json;

    use super::*;

    #[test]
    fn hostile_graph_text_is_escaped_everywhere() -> Result<(), Box<dyn Error>> {
        let graph: GraphDocument = serde_json::from_value(json!({
            "graph":{"hyperedges":[{"id":"h","label":"<img onerror=x>","nodes":["a","b"]}]},
            "nodes":[
                {"id":"a","label":"ApiClient","source_file":"src/api.py"},
                {"id":"b","label":"<script>alert(1)</script>","source_file":"src/evil.py"}
            ],
            "links":[{"source":"a","target":"b","relation":"calls","confidence":"EXTRACTED"}]
        }))?;
        let communities = BTreeMap::from([(0, vec!["a".into()]), (1, vec!["b".into()])]);
        let html = callflow_html_document(
            &graph,
            &communities,
            &CallflowOptions {
                report: "## God Nodes\n1. `<script>x</script>` - 1 edges",
                project_name: "<svg onload=x>",
                generated_at: Some("2026-07-19 12:00 UTC"),
                ..CallflowOptions::default()
            },
        )?;
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(html.contains("&lt;img onerror=x&gt;"));
        assert!(html.contains("&lt;svg onload=x&gt;"));
        assert!(!html.contains("<script>alert(1)</script>"));
        Ok(())
    }
}
