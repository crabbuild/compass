use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use compass_files::write_text_atomic;
use compass_model::GraphDocument;
use serde::{Deserialize, Serialize};

use crate::OutputError;
use crate::json::escape_non_ascii;

pub const DEFAULT_MAX_CHILDREN: isize = 200;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeNode {
    pub name: String,
    pub total_count: usize,
    pub children: Vec<TreeNode>,
}

#[derive(Clone, Debug)]
pub struct TreeOptions<'a> {
    pub root: Option<&'a Path>,
    pub max_children: isize,
    pub project_label: Option<&'a str>,
    pub svg_width: usize,
    pub svg_height: usize,
}

impl Default for TreeOptions<'_> {
    fn default() -> Self {
        Self {
            root: None,
            max_children: DEFAULT_MAX_CHILDREN,
            project_label: None,
            svg_width: 6_000,
            svg_height: 8_000,
        }
    }
}

#[must_use]
pub fn build_tree(document: &GraphDocument, options: &TreeOptions<'_>) -> TreeNode {
    let file_nodes = document
        .nodes
        .iter()
        .filter_map(|node| {
            let source = node
                .attributes
                .get("source_file")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            (!source.is_empty()).then_some((source, node))
        })
        .collect::<Vec<_>>();
    if file_nodes.is_empty() {
        return TreeNode {
            name: "(empty graph)".into(),
            total_count: 0,
            children: Vec::new(),
        };
    }
    let owned_root;
    let root = if let Some(root) = options.root {
        root
    } else {
        owned_root = common_root(file_nodes.iter().map(|(source, _)| *source));
        &owned_root
    };
    let root_text = root.to_string_lossy();
    let label = options
        .project_label
        .filter(|label| !label.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            root.file_name()
                .and_then(|name| name.to_str())
                .filter(|name| !name.is_empty())
                .map(ToOwned::to_owned)
        })
        .or_else(|| (!root_text.is_empty()).then(|| root_text.into_owned()))
        .unwrap_or_else(|| "/".into());
    let mut root_node = TreeNode {
        name: label,
        total_count: 0,
        children: Vec::new(),
    };

    let mut by_file = Vec::<(&str, Vec<&compass_model::NodeRecord>)>::new();
    for (source, node) in file_nodes {
        if let Some((_, nodes)) = by_file.iter_mut().find(|(existing, _)| *existing == source) {
            nodes.push(node);
        } else {
            by_file.push((source, vec![node]));
        }
    }
    by_file.sort_by(|left, right| left.0.cmp(right.0));
    for (source, symbols) in by_file {
        let source_path = Path::new(source);
        let directory_parts = source_path
            .strip_prefix(root)
            .ok()
            .and_then(Path::parent)
            .map(path_names)
            .unwrap_or_default();
        let parent = ensure_directory(&mut root_node, &directory_parts);
        let mut children = symbols
            .into_iter()
            .filter_map(|node| {
                let label = node
                    .attributes
                    .get("label")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(&node.id);
                let file_type = node
                    .attributes
                    .get("file_type")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                (label
                    != source_path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or_default()
                    || file_type != "code")
                    .then(|| TreeNode {
                        name: label.to_owned(),
                        total_count: 1,
                        children: Vec::new(),
                    })
            })
            .collect::<Vec<_>>();
        children.sort_by(|left, right| {
            (left.name.starts_with('_'), left.name.to_lowercase())
                .cmp(&(right.name.starts_with('_'), right.name.to_lowercase()))
        });
        let child_count = children.len();
        if i128::try_from(child_count).unwrap_or(i128::MAX) > options.max_children as i128 {
            let keep = if options.max_children >= 0 {
                usize::try_from(options.max_children)
                    .unwrap_or(usize::MAX)
                    .min(child_count)
            } else {
                child_count.saturating_sub(options.max_children.unsigned_abs())
            };
            let extra = if options.max_children >= 0 {
                child_count.saturating_sub(keep)
            } else {
                child_count.saturating_add(options.max_children.unsigned_abs())
            };
            children.truncate(keep);
            children.push(TreeNode {
                name: format!("(+{extra} more)"),
                total_count: extra,
                children: Vec::new(),
            });
        }
        parent.children.push(TreeNode {
            name: source_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(source)
                .to_owned(),
            total_count: children.len().max(1),
            children,
        });
    }
    finalize(&mut root_node);
    root_node
}

#[must_use]
pub fn tree_html_document(document: &GraphDocument, options: &TreeOptions<'_>) -> String {
    let tree = build_tree(document, options);
    emit_html(
        &tree,
        &format!("{} — graphify tree viewer", tree.name),
        &format!("{} — Knowledge Graph", tree.name),
        options.svg_width,
        options.svg_height,
    )
}

pub fn write_tree_html(
    document: &GraphDocument,
    output_path: impl AsRef<Path>,
    options: &TreeOptions<'_>,
) -> Result<(), OutputError> {
    write_text_atomic(output_path, &tree_html_document(document, options))?;
    Ok(())
}

fn common_root<'a>(paths: impl Iterator<Item = &'a str>) -> PathBuf {
    let mut paths = paths.map(Path::new);
    let Some(first) = paths.next() else {
        return PathBuf::new();
    };
    let mut common = first.components().collect::<Vec<_>>();
    for path in paths {
        let other = path.components().collect::<Vec<_>>();
        let shared = common
            .iter()
            .zip(other.iter())
            .take_while(|(left, right)| left == right)
            .count();
        common.truncate(shared);
    }
    common.into_iter().collect()
}

fn path_names(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|component| match component {
            std::path::Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect()
}

fn ensure_directory<'a>(node: &'a mut TreeNode, parts: &[String]) -> &'a mut TreeNode {
    let Some((name, rest)) = parts.split_first() else {
        return node;
    };
    let position = node
        .children
        .iter()
        .position(|child| child.name == *name)
        .unwrap_or_else(|| {
            node.children.push(TreeNode {
                name: name.clone(),
                total_count: 0,
                children: Vec::new(),
            });
            node.children.len() - 1
        });
    ensure_directory(&mut node.children[position], rest)
}

fn finalize(node: &mut TreeNode) -> usize {
    node.children.sort_by(|left, right| {
        let ordering = (!left.children.is_empty())
            .cmp(&!right.children.is_empty())
            .reverse();
        if ordering == Ordering::Equal {
            left.name.to_lowercase().cmp(&right.name.to_lowercase())
        } else {
            ordering
        }
    });
    if node.children.is_empty() {
        return node.total_count.max(1);
    }
    node.total_count = node.children.iter_mut().map(finalize).sum::<usize>().max(1);
    node.total_count
}

fn emit_html(
    tree: &TreeNode,
    title: &str,
    header: &str,
    svg_width: usize,
    svg_height: usize,
) -> String {
    let data = serde_json::to_string(tree)
        .map(|json| escape_non_ascii(&json).replace("</", "<\\/"))
        .unwrap_or_else(|_| "{}".into());
    let title = html_escape(title);
    let header = html_escape(header);
    let width = svg_width.to_string();
    let height = svg_height.to_string();
    render_template(
        include_str!("../assets/tree-template.html"),
        &[
            ("@@COMPASS_TREE_TITLE@@", title.as_str()),
            ("@@COMPASS_TREE_HEADER@@", header.as_str()),
            ("@@COMPASS_TREE_WIDTH@@", width.as_str()),
            ("@@COMPASS_TREE_HEIGHT@@", height.as_str()),
            ("@@COMPASS_TREE_DATA@@", data.as_str()),
        ],
    )
}

fn render_template(template: &str, replacements: &[(&str, &str)]) -> String {
    let mut output = String::with_capacity(template.len());
    let mut remaining = template;
    loop {
        let next = replacements
            .iter()
            .filter_map(|(marker, replacement)| {
                remaining
                    .find(marker)
                    .map(|position| (position, *marker, *replacement))
            })
            .min_by_key(|(position, _, _)| *position);
        let Some((position, marker, replacement)) = next else {
            output.push_str(remaining);
            return output;
        };
        output.push_str(&remaining[..position]);
        output.push_str(replacement);
        remaining = &remaining[position + marker.len()..];
    }
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use serde_json::json;

    use super::*;

    #[test]
    fn builds_sorted_truncated_hierarchy() -> Result<(), Box<dyn Error>> {
        let graph: GraphDocument = serde_json::from_value(json!({
            "nodes":[
                {"id":"file","label":"a.py","file_type":"code","source_file":"src/pkg/a.py"},
                {"id":"z","label":"zeta","file_type":"code","source_file":"src/pkg/a.py"},
                {"id":"a","label":"alpha","file_type":"code","source_file":"src/pkg/a.py"},
                {"id":"p","label":"_private","file_type":"code","source_file":"src/pkg/a.py"},
                {"id":"b","label":"Beta","file_type":"code","source_file":"src/b.py"}
            ],"links":[]
        }))?;
        let tree = build_tree(
            &graph,
            &TreeOptions {
                max_children: 2,
                ..TreeOptions::default()
            },
        );
        assert_eq!(tree.name, "src");
        assert_eq!(tree.total_count, 4);
        let package = tree
            .children
            .iter()
            .find(|child| child.name == "pkg")
            .ok_or("package directory missing")?;
        assert_eq!(package.name, "pkg");
        assert_eq!(package.children[0].children[0].name, "(+1 more)");
        assert_eq!(package.children[0].children[1].name, "alpha");
        Ok(())
    }

    #[test]
    fn emitted_data_and_headings_are_xss_safe() -> Result<(), Box<dyn Error>> {
        let graph: GraphDocument = serde_json::from_value(json!({
            "nodes":[{"id":"x","label":"</script><script>x()</script>","source_file":"x.py"}],
            "links":[]
        }))?;
        let html = tree_html_document(
            &graph,
            &TreeOptions {
                project_label: Some("</script><img src=x onerror=x()>"),
                ..TreeOptions::default()
            },
        );
        assert!(html.contains("&lt;/script&gt;&lt;img src=x onerror=x()&gt;"));
        assert!(html.contains("<\\/script>"));
        assert!(!html.contains("const initialJsonData={\"name\":\"</script>"));
        Ok(())
    }
}
