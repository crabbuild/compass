use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde_json::{Map, Value};
use sha1::{Digest, Sha1};
use trail_files::{FileError, write_text_atomic};
use trail_graph::Communities;
use trail_model::{EdgeRecord, GraphDocument, NodeRecord};

use crate::OutputError;
use crate::json::escape_non_ascii;

const COMMUNITY_COLORS: [&str; 10] = [
    "#4E79A7", "#F28E2B", "#E15759", "#76B7B2", "#59A14F", "#EDC948", "#B07AA1", "#FF9DA7",
    "#9C755F", "#BAB0AC",
];
const MANIFEST: &str = ".graphify_obsidian_manifest.json";

#[derive(Clone, Debug, Default)]
pub struct ObsidianOptions<'a> {
    pub community_labels: Option<&'a BTreeMap<usize, String>>,
    pub cohesion: Option<&'a BTreeMap<usize, f64>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ObsidianExport {
    pub notes_written: usize,
    pub pruned: usize,
    pub skipped: Vec<String>,
}

#[must_use]
pub fn node_filenames(document: &GraphDocument) -> BTreeMap<String, String> {
    let mut filenames = BTreeMap::new();
    let mut used = HashSet::new();
    for node in &document.nodes {
        let base = safe_note_name(node.label());
        let mut candidate = base.clone();
        let mut suffix = 1;
        while used.contains(&candidate.to_lowercase()) {
            candidate = format!("{base}_{suffix}");
            suffix += 1;
        }
        used.insert(candidate.to_lowercase());
        filenames.insert(node.id.clone(), candidate);
    }
    filenames
}

pub fn export_obsidian(
    document: &GraphDocument,
    communities: &Communities,
    output_dir: impl AsRef<Path>,
    options: &ObsidianOptions<'_>,
) -> Result<ObsidianExport, OutputError> {
    let out = output_dir.as_ref();
    fs::create_dir_all(out).map_err(|source| io_error(out, source))?;
    let manifest_path = out.join(MANIFEST);
    let owned = read_manifest(&manifest_path);
    let mut state = VaultState {
        out,
        owned,
        written: Vec::new(),
        skipped: Vec::new(),
    };
    let index = GraphIndex::new(document);
    let filenames = node_filenames(document);
    let node_community = community_map(communities);
    let mut node_notes_written = 0;

    for node in &document.nodes {
        let community = node_community.get(node.id.as_str()).copied();
        let name = community.map_or_else(
            || "Community None".to_owned(),
            |community| community_name(community, options.community_labels),
        );
        let file_type = string_attr(node, "file_type");
        let type_tag = match file_type.as_str() {
            "code" => "graphify/code".to_owned(),
            "document" => "graphify/document".to_owned(),
            "paper" => "graphify/paper".to_owned(),
            "image" => "graphify/image".to_owned(),
            "" => "graphify/document".to_owned(),
            other => format!("graphify/{other}"),
        };
        let confidence = dominant_confidence(&index, &node.id);
        let tags = [
            type_tag,
            format!("graphify/{confidence}"),
            format!("community/{}", obsidian_tag(&name)),
        ];
        let mut lines = vec![
            "---".to_owned(),
            format!(
                "source_file: \"{}\"",
                yaml_string(&string_attr(node, "source_file"))
            ),
            format!("type: \"{}\"", yaml_string(&file_type)),
            format!("community: \"{}\"", yaml_string(&name)),
        ];
        let location = string_attr(node, "source_location");
        if !location.is_empty() {
            lines.push(format!("location: \"{}\"", yaml_string(&location)));
        }
        lines.push("tags:".to_owned());
        lines.extend(tags.iter().map(|tag| format!("  - {tag}")));
        lines.extend([
            "---".to_owned(),
            String::new(),
            format!("# {}", node.label()),
            String::new(),
        ]);

        let mut neighbors = index.neighbors(&node.id);
        neighbors.sort_by(|left, right| {
            index
                .node(left)
                .map(NodeRecord::label)
                .cmp(&index.node(right).map(NodeRecord::label))
        });
        if !neighbors.is_empty() {
            lines.push("## Connections".to_owned());
            for neighbor in neighbors {
                let Some(filename) = filenames.get(&neighbor) else {
                    continue;
                };
                let edge = index.edge_between(&node.id, &neighbor);
                let relation =
                    edge.map_or_else(String::new, |edge| string_edge_attr(edge, "relation"));
                let confidence = edge.map_or_else(
                    || "EXTRACTED".to_owned(),
                    |edge| defaulted_edge_attr(edge, "confidence", "EXTRACTED"),
                );
                lines.push(format!("- [[{filename}]] - `{relation}` [{confidence}]"));
            }
            lines.push(String::new());
        }
        lines.push(
            tags.iter()
                .map(|tag| format!("#{tag}"))
                .collect::<Vec<_>>()
                .join(" "),
        );
        let relative = format!("{}.md", filenames[&node.id]);
        if state.owned_write(&relative, &lines.join("\n"))? {
            node_notes_written += 1;
        }
    }

    let mut cross_counts = communities
        .keys()
        .map(|id| (*id, BTreeMap::new()))
        .collect::<BTreeMap<_, BTreeMap<_, usize>>>();
    for edge in &document.links {
        let (Some(left), Some(right)) = (
            node_community.get(edge.source.as_str()),
            node_community.get(edge.target.as_str()),
        ) else {
            continue;
        };
        if left != right {
            *cross_counts
                .entry(*left)
                .or_default()
                .entry(*right)
                .or_default() += 1;
            *cross_counts
                .entry(*right)
                .or_default()
                .entry(*left)
                .or_default() += 1;
        }
    }
    let mut community_filenames = BTreeMap::new();
    let mut used = HashSet::new();
    for community in communities.keys() {
        let base = format!(
            "_COMMUNITY_{}",
            safe_note_name(&community_name(*community, options.community_labels))
        );
        let mut candidate = base.clone();
        let mut suffix = 1;
        while used.contains(&candidate.to_lowercase()) {
            candidate = format!("{base}_{suffix}");
            suffix += 1;
        }
        used.insert(candidate.to_lowercase());
        community_filenames.insert(*community, candidate);
    }

    let mut community_notes_written = 0;
    for (community, source_members) in communities {
        let name = community_name(*community, options.community_labels);
        let mut members = source_members
            .iter()
            .filter(|member| index.node(member).is_some() && filenames.contains_key(*member))
            .collect::<Vec<_>>();
        let cohesion = options
            .cohesion
            .and_then(|values| values.get(community))
            .copied();
        let mut lines = vec!["---".to_owned(), "type: community".to_owned()];
        if let Some(value) = cohesion {
            lines.push(format!("cohesion: {value:.2}"));
        }
        lines.extend([
            format!("members: {}", members.len()),
            "---".to_owned(),
            String::new(),
            format!("# {name}"),
            String::new(),
        ]);
        if let Some(value) = cohesion {
            let description = if value >= 0.7 {
                "tightly connected"
            } else if value >= 0.4 {
                "moderately connected"
            } else {
                "loosely connected"
            };
            lines.push(format!("**Cohesion:** {value:.2} - {description}"));
        }
        lines.extend([
            format!("**Members:** {} nodes", members.len()),
            String::new(),
            "## Members".to_owned(),
        ]);
        members.sort_by(|left, right| {
            index
                .node(left)
                .map(NodeRecord::label)
                .cmp(&index.node(right).map(NodeRecord::label))
        });
        for member in &members {
            let Some(node) = index.node(member) else {
                continue;
            };
            let mut entry = format!("- [[{}]]", filenames[*member]);
            let file_type = string_attr(node, "file_type");
            let source = string_attr(node, "source_file");
            if !file_type.is_empty() {
                entry.push_str(&format!(" - {file_type}"));
            }
            if !source.is_empty() {
                entry.push_str(&format!(" - {source}"));
            }
            lines.push(entry);
        }
        lines.extend([
            String::new(),
            "## Live Query (requires Dataview plugin)".to_owned(),
            String::new(),
            "```dataview".to_owned(),
            format!(
                "TABLE source_file, type FROM #community/{}",
                obsidian_tag(&name)
            ),
            "SORT file.name ASC".to_owned(),
            "```".to_owned(),
            String::new(),
        ]);
        if let Some(cross) = cross_counts
            .get(community)
            .filter(|cross| !cross.is_empty())
        {
            lines.push("## Connections to other communities".to_owned());
            let mut counts = cross.iter().collect::<Vec<_>>();
            counts.sort_by_key(|(_, count)| std::cmp::Reverse(**count));
            for (other, count) in counts {
                let other_name = community_filenames.get(other).cloned().unwrap_or_else(|| {
                    format!(
                        "_COMMUNITY_{}",
                        safe_note_name(&community_name(*other, options.community_labels))
                    )
                });
                let plural = if *count == 1 { "" } else { "s" };
                lines.push(format!("- {count} edge{plural} to [[{other_name}]]"));
            }
            lines.push(String::new());
        }
        let mut bridges = members
            .iter()
            .filter_map(|member| {
                let reach = community_reach(&index, member, &node_community);
                (reach > 0).then(|| ((*member).clone(), index.degree(member), reach))
            })
            .collect::<Vec<_>>();
        bridges.sort_by_key(|(_, degree, reach)| {
            (std::cmp::Reverse(*reach), std::cmp::Reverse(*degree))
        });
        if !bridges.is_empty() {
            lines.push("## Top bridge nodes".to_owned());
            for (member, degree, reach) in bridges.into_iter().take(5) {
                let plural = if reach == 1 {
                    "community"
                } else {
                    "communities"
                };
                lines.push(format!(
                    "- [[{}]] - degree {degree}, connects to {reach} {plural}",
                    filenames[&member]
                ));
            }
        }
        let relative = format!("{}.md", community_filenames[community]);
        if state.owned_write(&relative, &lines.join("\n"))? {
            community_notes_written += 1;
        }
    }

    let mut color_groups = Vec::new();
    if let Some(labels) = options.community_labels {
        for (community, label) in labels {
            let rgb = u64::from_str_radix(
                COMMUNITY_COLORS[community % COMMUNITY_COLORS.len()].trim_start_matches('#'),
                16,
            )
            .unwrap_or_default();
            color_groups.push(serde_json::json!({
                "query": format!("tag:#community/{}", label.replace(' ', "_")),
                "color": {"a": 1, "rgb": rgb}
            }));
        }
    }
    let mut graph_config = Map::new();
    graph_config.insert("colorGroups".to_owned(), Value::Array(color_groups));
    let config = escape_non_ascii(
        &serde_json::to_string_pretty(&Value::Object(graph_config)).unwrap_or_default(),
    );
    state.owned_write(".obsidian/graph.json", &config)?;

    let written = state.written.iter().cloned().collect::<HashSet<_>>();
    let skipped = state.skipped.iter().cloned().collect::<HashSet<_>>();
    let stale = state
        .owned
        .difference(&written)
        .filter(|item| !skipped.contains(*item))
        .cloned()
        .collect::<Vec<_>>();
    let mut pruned = 0;
    for relative in stale {
        if let Some(target) = contained_target(out, &relative) {
            match fs::remove_file(&target) {
                Ok(()) => pruned += 1,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => {}
            }
        }
    }
    let mut files = state.written.clone();
    files.sort();
    files.dedup();
    let manifest = serde_json::json!({"files": files});
    let encoded = escape_non_ascii(&serde_json::to_string_pretty(&manifest).unwrap_or_default());
    let _ = write_text_atomic(&manifest_path, &encoded);
    Ok(ObsidianExport {
        notes_written: node_notes_written + community_notes_written,
        pruned,
        skipped: state.skipped,
    })
}

pub(crate) fn safe_note_name(label: &str) -> String {
    let normalized = label.replace("\r\n", " ").replace(['\r', '\n'], " ");
    let mut cleaned = normalized
        .chars()
        .filter(|character| {
            !matches!(
                character,
                '\\' | '/' | '*' | '?' | ':' | '"' | '<' | '>' | '|' | '#' | '^' | '[' | ']'
            )
        })
        .collect::<String>();
    cleaned = cleaned.trim().to_owned();
    let lowercase = cleaned.to_lowercase();
    if let Some(extension) = [".markdown", ".mdx", ".qmd", ".md"]
        .into_iter()
        .find(|extension| lowercase.ends_with(extension))
    {
        cleaned.truncate(cleaned.len() - extension.len());
    }
    if !cleaned
        .chars()
        .any(|character| character == '_' || character.is_alphanumeric())
    {
        return "unnamed".to_owned();
    }
    cap_filename(&cleaned, 200)
}

fn cap_filename(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_owned();
    }
    let digest = format!("{:x}", Sha1::digest(value.as_bytes()));
    let mut keep = limit - 9;
    while !value.is_char_boundary(keep) {
        keep -= 1;
    }
    format!("{}_{}", &value[..keep], &digest[..8])
}

fn obsidian_tag(name: &str) -> String {
    name.replace(' ', "_")
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '/')
        })
        .collect()
}

fn yaml_string(value: &str) -> String {
    let mut output = String::new();
    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '\0' => output.push_str("\\0"),
            '\u{2028}' => output.push_str("\\L"),
            '\u{2029}' => output.push_str("\\P"),
            control if (control as u32) < 0x20 || control == '\u{7f}' => {
                output.push_str(&format!("\\x{:02x}", control as u32));
            }
            other => output.push(other),
        }
    }
    output
}

fn community_name(community: usize, labels: Option<&BTreeMap<usize, String>>) -> String {
    labels
        .and_then(|labels| labels.get(&community).cloned())
        .unwrap_or_else(|| format!("Community {community}"))
}

fn community_map(communities: &Communities) -> HashMap<&str, usize> {
    communities
        .iter()
        .flat_map(|(community, members)| {
            members
                .iter()
                .map(move |member| (member.as_str(), *community))
        })
        .collect()
}

fn string_attr(node: &NodeRecord, key: &str) -> String {
    node.string(key)
}

fn string_edge_attr(edge: &EdgeRecord, key: &str) -> String {
    edge.string(key)
}

fn defaulted_edge_attr(edge: &EdgeRecord, key: &str, default: &str) -> String {
    let value = string_edge_attr(edge, key);
    if value.is_empty() {
        default.to_owned()
    } else {
        value
    }
}

struct GraphIndex<'a> {
    document: &'a GraphDocument,
    nodes: HashMap<&'a str, &'a NodeRecord>,
}

impl<'a> GraphIndex<'a> {
    fn new(document: &'a GraphDocument) -> Self {
        Self {
            document,
            nodes: document
                .nodes
                .iter()
                .map(|node| (node.id.as_str(), node))
                .collect(),
        }
    }

    fn node(&self, id: &str) -> Option<&'a NodeRecord> {
        self.nodes.get(id).copied()
    }

    fn neighbors(&self, id: &str) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut output = Vec::new();
        for edge in &self.document.links {
            let neighbor = if edge.source == id {
                Some(&edge.target)
            } else if !self.document.directed && edge.target == id {
                Some(&edge.source)
            } else {
                None
            };
            if let Some(neighbor) = neighbor.filter(|neighbor| seen.insert((*neighbor).clone())) {
                output.push(neighbor.clone());
            }
        }
        output
    }

    fn incident_edges(&self, id: &str) -> Vec<&'a EdgeRecord> {
        let mut edges = self
            .document
            .links
            .iter()
            .filter(|edge| edge.source == id || (!self.document.directed && edge.target == id))
            .collect::<Vec<_>>();
        edges.sort_by(|left, right| edge_insertion_key(left).cmp(&edge_insertion_key(right)));
        edges
    }

    fn edge_between(&self, source: &str, target: &str) -> Option<&'a EdgeRecord> {
        self.document.links.iter().rev().find(|edge| {
            (edge.source == source && edge.target == target)
                || (!self.document.directed && edge.source == target && edge.target == source)
        })
    }

    fn degree(&self, id: &str) -> usize {
        self.document
            .links
            .iter()
            .filter(|edge| edge.source == id || edge.target == id)
            .count()
    }
}

fn edge_insertion_key(edge: &EdgeRecord) -> (&str, &str, &str) {
    (
        edge.attributes
            .get("_src")
            .and_then(Value::as_str)
            .unwrap_or(&edge.source),
        edge.attributes
            .get("_tgt")
            .and_then(Value::as_str)
            .unwrap_or(&edge.target),
        edge.attributes
            .get("relation")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    )
}

fn dominant_confidence(index: &GraphIndex<'_>, id: &str) -> String {
    let mut counts = HashMap::<String, usize>::new();
    let mut order = Vec::new();
    for edge in index.incident_edges(id) {
        let confidence = defaulted_edge_attr(edge, "confidence", "EXTRACTED");
        if !counts.contains_key(&confidence) {
            order.push(confidence.clone());
        }
        *counts.entry(confidence).or_default() += 1;
    }
    let mut best = None;
    let mut best_count = 0;
    for confidence in order {
        let count = counts.get(&confidence).copied().unwrap_or_default();
        if count > best_count {
            best = Some(confidence);
            best_count = count;
        }
    }
    best.unwrap_or_else(|| "EXTRACTED".to_owned())
}

fn community_reach(index: &GraphIndex<'_>, id: &str, communities: &HashMap<&str, usize>) -> usize {
    let own = communities.get(id);
    index
        .neighbors(id)
        .iter()
        .filter_map(|neighbor| communities.get(neighbor.as_str()))
        .filter(|community| Some(*community) != own)
        .collect::<HashSet<_>>()
        .len()
}

struct VaultState<'a> {
    out: &'a Path,
    owned: HashSet<String>,
    written: Vec<String>,
    skipped: Vec<String>,
}

impl VaultState<'_> {
    fn owned_write(&mut self, relative: &str, content: &str) -> Result<bool, OutputError> {
        let Some(target) = contained_target(self.out, relative) else {
            return Err(OutputError::InvalidObsidianPath(PathBuf::from(relative)));
        };
        if target.exists() && !self.owned.contains(relative) {
            self.skipped.push(relative.to_owned());
            return Ok(false);
        }
        write_text_atomic(target, content)?;
        self.written.push(relative.to_owned());
        Ok(true)
    }
}

fn read_manifest(path: &Path) -> HashSet<String> {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|value| value.get("files").and_then(Value::as_array).cloned())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| value.as_str().map(ToOwned::to_owned))
        .collect()
}

fn contained_target(root: &Path, relative: &str) -> Option<PathBuf> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return None;
    }
    let target = root.join(path);
    let canonical_root = fs::canonicalize(root).ok()?;
    let mut existing = target.as_path();
    while !existing.exists() {
        existing = existing.parent()?;
    }
    let canonical_existing = fs::canonicalize(existing).ok()?;
    if canonical_existing == canonical_root || canonical_existing.starts_with(&canonical_root) {
        Some(target)
    } else {
        None
    }
}

fn io_error(path: impl Into<PathBuf>, source: std::io::Error) -> OutputError {
    OutputError::File(FileError::Io {
        path: path.into(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use serde_json::json;
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn filenames_are_case_safe_suffix_safe_and_byte_capped() -> Result<(), Box<dyn Error>> {
        let graph: GraphDocument = serde_json::from_value(json!({
            "nodes": [
                {"id":"a", "label":"References"},
                {"id":"b", "label":"references"},
                {"id":"c", "label":"References_1"},
                {"id":"d", "label":"@/*"},
                {"id":"e", "label":"界".repeat(100)}
            ],
            "links": []
        }))?;
        let names = node_filenames(&graph);
        assert_eq!(names["a"], "References");
        assert_eq!(names["b"], "references_1");
        assert_eq!(names["c"], "References_1_1");
        assert_eq!(names["d"], "unnamed");
        assert!(names["e"].len() <= 200);
        assert_eq!(
            names["e"]
                .chars()
                .filter(|character| *character == '_')
                .count(),
            1
        );
        Ok(())
    }

    #[test]
    fn preserves_user_files_and_escapes_yaml() -> Result<(), Box<dyn Error>> {
        let directory = tempdir()?;
        let vault = directory.path();
        fs::create_dir(vault.join(".obsidian"))?;
        fs::write(vault.join("Database.md"), "# MY NOTES\n")?;
        fs::write(vault.join(".obsidian/graph.json"), "{\"USER\":true}")?;
        let graph = two_nodes("bad\"\nadmin: true");
        let result = export_obsidian(
            &graph,
            &BTreeMap::from([(0, vec!["n1".into(), "n2".into()])]),
            vault,
            &ObsidianOptions {
                community_labels: Some(&BTreeMap::from([(0, "Backend".into())])),
                cohesion: None,
            },
        )?;
        assert_eq!(
            fs::read_to_string(vault.join("Database.md"))?,
            "# MY NOTES\n"
        );
        assert_eq!(
            fs::read_to_string(vault.join(".obsidian/graph.json"))?,
            "{\"USER\":true}"
        );
        assert_eq!(result.skipped.len(), 2);
        let server = fs::read_to_string(vault.join("Server.md"))?;
        assert!(server.contains("source_file: \"bad\\\"\\nadmin: true\""));
        Ok(())
    }

    #[test]
    fn prunes_owned_notes_and_allows_returning_nodes() -> Result<(), Box<dyn Error>> {
        let directory = tempdir()?;
        let vault = directory.path();
        let both = two_nodes("app/server.py");
        let both_communities = BTreeMap::from([(0, vec!["n1".into(), "n2".into()])]);
        export_obsidian(&both, &both_communities, vault, &ObsidianOptions::default())?;
        assert!(vault.join("Server.md").exists());
        let one: GraphDocument = serde_json::from_value(json!({
            "nodes": [{"id":"n1", "label":"Database", "file_type":"code"}],
            "links": []
        }))?;
        let result = export_obsidian(
            &one,
            &BTreeMap::from([(0, vec!["n1".into()])]),
            vault,
            &ObsidianOptions::default(),
        )?;
        assert!(result.pruned >= 1);
        assert!(!vault.join("Server.md").exists());
        let result = export_obsidian(&both, &both_communities, vault, &ObsidianOptions::default())?;
        assert!(result.skipped.is_empty());
        assert!(vault.join("Server.md").exists());
        Ok(())
    }

    #[test]
    fn hostile_manifest_cannot_prune_outside_vault() -> Result<(), Box<dyn Error>> {
        let directory = tempdir()?;
        let vault = directory.path().join("vault");
        fs::create_dir(&vault)?;
        let outside = directory.path().join("outside.md");
        fs::write(&outside, "safe")?;
        fs::write(vault.join(MANIFEST), "{\"files\":[\"../outside.md\"]}")?;
        let graph: GraphDocument = serde_json::from_value(json!({"nodes":[],"links":[]}))?;
        export_obsidian(
            &graph,
            &Communities::new(),
            &vault,
            &ObsidianOptions::default(),
        )?;
        assert_eq!(fs::read_to_string(outside)?, "safe");
        Ok(())
    }

    fn two_nodes(source_file: &str) -> GraphDocument {
        serde_json::from_value(json!({
            "nodes": [
                {"id":"n1", "label":"Database", "file_type":"code", "source_file":"app/db.py"},
                {"id":"n2", "label":"Server", "file_type":"code", "source_file":source_file}
            ],
            "links": [
                {"source":"n1", "target":"n2", "relation":"calls", "confidence":"EXTRACTED"}
            ]
        }))
        .unwrap_or_else(|_| GraphDocument {
            directed: false,
            multigraph: false,
            graph: Map::new(),
            nodes: Vec::new(),
            links: Vec::new(),
            extras: BTreeMap::new(),
            used_legacy_edges_key: false,
        })
    }
}
