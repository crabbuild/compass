use std::error::Error;
use std::fs;
use std::path::Path;

use serde_json::Value;
use trail_languages::{Engine, Extraction};

fn extract(path: &Path, contents: impl AsRef<[u8]>) -> Result<Extraction, Box<dyn Error>> {
    fs::write(path, contents)?;
    Ok(Engine::default().extract(path)?)
}

fn version(extraction: &Extraction) -> Option<&Value> {
    extraction
        .nodes
        .first()
        .and_then(|node| node.attributes.get("version"))
}

#[test]
fn apm_manifests_cover_scalar_versions_dependency_shapes_and_empty_inputs()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("apm.yml");

    let mapping = extract(
        &path,
        "name: fixture\nversion: true\ndependencies:\n  alpha: 1\n  false: 2\n  7: 3\n  null: 4\n  nested: {}\n",
    )?;
    assert_eq!(mapping.nodes.len(), 1);
    assert_eq!(version(&mapping), Some(&Value::Bool(true)));
    assert_eq!(mapping.edges.len(), 5);

    let sequence = extract(
        &path,
        "name: fixture\nversion: 1.25\ndependencies:\n  - alpha\n  - beta: latest\n  - true\n  - [ignored]\n",
    )?;
    assert!(version(&sequence).is_some_and(Value::is_number));
    assert_eq!(sequence.edges.len(), 2);

    for contents in [
        "- not-a-mapping\n",
        "version: 1\n",
        "name: 7\n",
        "name: fixture\nversion: null\ndependencies: scalar\n",
    ] {
        let result = extract(&path, contents)?;
        if contents.starts_with("name: fixture") {
            assert_eq!(result.nodes.len(), 1);
            assert!(version(&result).is_none());
        } else {
            assert!(result.nodes.is_empty());
        }
    }
    assert!(
        extract(&path, "name: [broken\n")?
            .error
            .as_deref()
            .is_some_and(|error| error.starts_with("manifest parse error:"))
    );
    Ok(())
}

#[test]
fn pyproject_manifests_cover_pep508_poetry_scalar_versions_and_parse_failures()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("pyproject.toml");

    let project = extract(
        &path,
        r#"[project]
name = "fixture"
version = 7
dependencies = ["requests>=2", "typing-extensions ; python_version < '3.11'", "bad", 4]
"#,
    )?;
    assert_eq!(version(&project), Some(&Value::from(7)));
    assert_eq!(project.edges.len(), 3);

    let poetry = extract(
        &path,
        r#"[tool.poetry]
name = "poetry-fixture"
version = 2026-07-20T12:34:56Z
[tool.poetry.dependencies]
python = "^3.12"
httpx = "*"
rich = { version = "*" }
"#,
    )?;
    assert!(version(&poetry).is_some_and(Value::is_string));
    assert_eq!(poetry.edges.len(), 2);

    for value in ["true", "1.5", "[]", "{}"] {
        let result = extract(
            &path,
            format!("[project]\nname = \"fixture\"\nversion = {value}\n"),
        )?;
        if matches!(value, "true" | "1.5") {
            assert!(version(&result).is_some());
        } else {
            assert!(version(&result).is_none());
        }
    }
    assert!(extract(&path, "[project]\nversion = 1\n")?.nodes.is_empty());
    assert!(extract(&path, "[project\n")?.error.is_some());
    Ok(())
}

#[test]
fn go_and_maven_manifests_cover_dependency_forms_missing_fields_and_size_limit()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let go = directory.path().join("go.mod");
    let module = extract(
        &go,
        r#"module example.test/root

require direct.test/mod v1.2.3
require invalid.test/mod latest
require (
  block.test/one v0.1.0
  block.test/two latest
)
"#,
    )?;
    assert_eq!(module.nodes.len(), 1);
    assert_eq!(module.edges.len(), 2);
    assert!(extract(&go, "go 1.23\n")?.nodes.is_empty());

    let pom = directory.path().join("pom.xml");
    let maven = extract(
        &pom,
        r#"<project>
  <groupId>org.fixture</groupId><artifactId>root</artifactId><version>1</version>
  <dependencies>
    <dependency><groupId>org.alpha</groupId><artifactId>a</artifactId></dependency>
    <dependency><artifactId>b</artifactId></dependency>
    <dependency><groupId>ignored</groupId></dependency>
  </dependencies>
</project>"#,
    )?;
    assert_eq!(maven.nodes[0].label(), "org.fixture:root");
    assert_eq!(maven.edges.len(), 2);
    let groupless = extract(&pom, "<project><artifactId>root</artifactId></project>")?;
    assert_eq!(groupless.nodes[0].label(), "root");
    assert!(extract(&pom, "<project />")?.nodes.is_empty());
    assert!(extract(&pom, "<project>")?.error.is_some());

    assert_eq!(
        extract(&go, vec![b'x'; 2_000_001])?.error.as_deref(),
        Some("manifest too large to index")
    );
    Ok(())
}
