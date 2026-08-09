use std::error::Error;
use std::fs;

use compass_languages::Engine;

#[test]
fn framework_import_aliases_retain_the_import_statement_range() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("module.js");
    let source = concat!(
        "import * as NewModule from \"./module_test.js\";\n",
        "import * as m from \"./module_test.js\";\n",
        "import * as m from \"./module_test.js\";\n",
    );
    fs::write(&path, source)?;

    let extraction = Engine::default().extract(&path)?;
    let mut imports = extraction
        .nodes
        .iter()
        .filter(|node| {
            node.attributes
                .get("extractor")
                .and_then(serde_json::Value::as_str)
                == Some("compass.frameworks.typescript.imports")
        })
        .collect::<Vec<_>>();
    imports.sort_by_key(|node| node.attributes["local_name"].as_str());

    assert_eq!(
        imports
            .iter()
            .filter_map(|node| node.attributes["local_name"].as_str())
            .collect::<Vec<_>>(),
        ["NewModule", "m"]
    );
    for node in imports {
        assert_eq!(node.attributes["_origin"], "ast");
        assert!(
            node.attributes["source_file"]
                .as_str()
                .is_some_and(|source_file| source_file.ends_with("module.js"))
        );
        let start = usize::try_from(
            node.attributes["start_byte"]
                .as_u64()
                .ok_or("missing import start byte")?,
        )?;
        let end = usize::try_from(
            node.attributes["end_byte"]
                .as_u64()
                .ok_or("missing import end byte")?,
        )?;
        assert!(start < end);
        assert!(
            node.attributes["line_start"]
                .as_u64()
                .is_some_and(|line| line > 0)
        );
        assert!(
            node.attributes["line_end"]
                .as_u64()
                .is_some_and(|line| line > 0)
        );
        assert!(node.attributes["column_start"].is_u64());
        assert!(node.attributes["column_end"].is_u64());

        let statement = source
            .get(start..end)
            .ok_or("import range exceeds source")?;
        let local = node.attributes["local_name"]
            .as_str()
            .ok_or("missing local import name")?;
        assert!(statement.starts_with("import "));
        assert!(statement.contains(local));
        assert!(statement.contains("./module_test.js"));
    }
    Ok(())
}
