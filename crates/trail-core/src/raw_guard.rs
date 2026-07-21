use std::path::Path;

use trail_model::GraphDocument;

use crate::pipeline::{CoreError, SemanticLayer, semantic_is_incomplete};

pub(super) fn enforce_incomplete_raw_guard(
    semantic: Option<&SemanticLayer>,
    graph_path: &Path,
    root: &Path,
    new_count: usize,
) -> Result<(), CoreError> {
    let Some(layer) = semantic else {
        return Ok(());
    };
    if layer.allow_partial || !semantic_is_incomplete(layer, root) || !graph_path.exists() {
        return Ok(());
    }
    let existing = GraphDocument::load(graph_path)
        .map_err(|_| CoreError::IncompleteSemanticExisting(graph_path.to_path_buf()))?
        .nodes
        .len();
    if new_count < existing {
        return Err(CoreError::IncompleteSemanticShrink {
            existing,
            new: new_count,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fs;

    use serde_json::json;

    use super::*;

    fn layer(root: &Path, allow_partial: bool, incomplete: bool) -> SemanticLayer {
        SemanticLayer {
            fragment: json!({"nodes": [], "edges": []}),
            refreshed_files: Vec::new(),
            partial_files: incomplete
                .then(|| root.join("partial.md"))
                .into_iter()
                .collect(),
            allow_partial,
        }
    }

    #[test]
    fn raw_guard_covers_bypass_corruption_shrink_and_boundary_cases() -> Result<(), Box<dyn Error>>
    {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        let graph = root.join("graph.json");

        enforce_incomplete_raw_guard(None, &graph, root, 0)?;
        enforce_incomplete_raw_guard(Some(&layer(root, false, false)), &graph, root, 0)?;
        enforce_incomplete_raw_guard(Some(&layer(root, false, true)), &graph, root, 0)?;

        fs::write(&graph, "not-json")?;
        enforce_incomplete_raw_guard(Some(&layer(root, true, true)), &graph, root, 0)?;
        assert!(matches!(
            enforce_incomplete_raw_guard(Some(&layer(root, false, true)), &graph, root, 0),
            Err(CoreError::IncompleteSemanticExisting(_))
        ));

        fs::write(
            &graph,
            serde_json::to_vec(&json!({
                "directed": true,
                "multigraph": false,
                "graph": {},
                "nodes": [{"id": "one"}, {"id": "two"}],
                "links": []
            }))?,
        )?;
        assert!(matches!(
            enforce_incomplete_raw_guard(Some(&layer(root, false, true)), &graph, root, 1),
            Err(CoreError::IncompleteSemanticShrink {
                existing: 2,
                new: 1
            })
        ));
        enforce_incomplete_raw_guard(Some(&layer(root, false, false)), &graph, root, 0)?;
        enforce_incomplete_raw_guard(Some(&layer(root, false, true)), &graph, root, 2)?;
        enforce_incomplete_raw_guard(Some(&layer(root, false, true)), &graph, root, 3)?;
        Ok(())
    }
}
