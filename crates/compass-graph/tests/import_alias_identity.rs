use std::error::Error;
use std::fs;

use compass_graph::{build_from_extraction, normalize_document_v1};
use compass_languages::Engine;
use compass_model::code_graph::{NodeDetails, NodeKind};

#[test]
fn namespace_imports_from_one_module_keep_distinct_local_alias_identities()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let path = root.join("module.js");
    fs::write(
        &path,
        r#"
import * as NewModule from "./module_test.js";
import * as m from "./module_test.js";
import * as m from "./module_test.js";
"#,
    )?;

    let extraction = Engine::default().extract(&path)?;
    let flexible = build_from_extraction(&extraction, true, Some(root));
    let graph = normalize_document_v1(&flexible, root, "sha256:test", None)?;

    let mut imports = graph
        .nodes
        .iter()
        .filter_map(|node| {
            if node.kind != NodeKind::Import {
                return None;
            }
            let Some(NodeDetails::ImportExport(details)) = &node.details else {
                return None;
            };
            Some((
                details.local_name.as_deref().unwrap_or_default(),
                node.id.as_str(),
            ))
        })
        .collect::<Vec<_>>();
    imports.sort_unstable();
    assert_eq!(
        imports.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
        ["NewModule", "m"]
    );
    assert_ne!(imports[0].1, imports[1].1);
    Ok(())
}
