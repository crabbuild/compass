use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{Value, json};

const DISTRIBUTION_SCHEMA: &str = "compass.distribution/1";
const INVENTORY: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../distribution.toml"
));
const OPENCODE_RUNTIME: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../packages/compass-opencode/src/index.js"
));

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DistributionInventory {
    schema: String,
    package: PackageInventory,
    harness: BTreeMap<String, HarnessInventory>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PackageInventory {
    name: String,
    version: String,
    description: String,
    author: String,
    homepage: String,
    repository: String,
    license: String,
    keywords: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HarnessInventory {
    verified_version: String,
    manifest: String,
    mcp: String,
    marketplace: Option<String>,
    npm_name: Option<String>,
    plugin_api_version: Option<String>,
}

pub(crate) struct NativePackage {
    pub files: BTreeMap<String, Vec<u8>>,
}

pub(crate) fn native_package(platform: &str, transport: &str) -> Result<NativePackage, String> {
    let inventory = load_inventory()?;
    let harness = inventory
        .harness
        .get(platform)
        .ok_or_else(|| format!("error: distribution inventory has no '{platform}' harness"))?;
    let mut files = BTreeMap::new();
    let mcp = mcp_config(platform, transport, harness.npm_name.as_deref())?;
    insert_json(&mut files, &harness.mcp, &mcp)?;
    match platform {
        "codex" => {
            insert_json(
                &mut files,
                &harness.manifest,
                &plugin_manifest(&inventory.package, harness, true),
            )?;
            let marketplace = harness.marketplace.as_deref().ok_or_else(|| {
                "error: Codex distribution is missing its marketplace path".to_owned()
            })?;
            insert_json(
                &mut files,
                marketplace,
                &codex_marketplace_manifest(&inventory.package),
            )?;
        }
        "claude" => {
            insert_json(
                &mut files,
                &harness.manifest,
                &plugin_manifest(&inventory.package, harness, false),
            )?;
            let marketplace = harness.marketplace.as_deref().ok_or_else(|| {
                "error: Claude distribution is missing its marketplace path".to_owned()
            })?;
            insert_json(
                &mut files,
                marketplace,
                &marketplace_manifest(&inventory.package),
            )?;
        }
        "opencode" => {
            insert_json(
                &mut files,
                &harness.manifest,
                &opencode_manifest(&inventory.package, harness)?,
            )?;
            files.insert(
                "src/index.js".to_owned(),
                OPENCODE_RUNTIME.as_bytes().to_vec(),
            );
            files.insert(
                "README.md".to_owned(),
                opencode_readme(&inventory.package, harness).into_bytes(),
            );
        }
        _ => {
            return Err(format!(
                "error: platform '{platform}' does not have a native package generator"
            ));
        }
    }
    Ok(NativePackage { files })
}

pub(crate) fn verified_harness_version(platform: &str) -> Result<String, String> {
    load_inventory()?
        .harness
        .get(platform)
        .map(|harness| harness.verified_version.clone())
        .ok_or_else(|| format!("error: distribution inventory has no '{platform}' harness"))
}

fn load_inventory() -> Result<DistributionInventory, String> {
    let inventory = toml::from_str::<DistributionInventory>(INVENTORY)
        .map_err(|error| format!("error: invalid distribution.toml: {error}"))?;
    if inventory.schema != DISTRIBUTION_SCHEMA {
        return Err(format!(
            "error: unsupported distribution schema '{}'",
            inventory.schema
        ));
    }
    if inventory.package.name != "compass" || inventory.package.version != env!("CARGO_PKG_VERSION")
    {
        return Err(format!(
            "error: distribution package identity must be compass {}",
            env!("CARGO_PKG_VERSION")
        ));
    }
    for required in ["codex", "claude", "opencode"] {
        let harness = inventory.harness.get(required).ok_or_else(|| {
            format!("error: distribution inventory is missing harness '{required}'")
        })?;
        for (field, path) in [("manifest", &harness.manifest), ("mcp", &harness.mcp)] {
            validate_relative_path(field, path)?;
        }
        if let Some(path) = &harness.marketplace {
            validate_relative_path("marketplace", path)?;
        }
    }
    Ok(inventory)
}

fn validate_relative_path(field: &str, path: &str) -> Result<(), String> {
    let parsed = std::path::Path::new(path);
    if path.is_empty()
        || parsed.is_absolute()
        || parsed.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Err(format!(
            "error: distribution {field} path '{path}' is not portable"
        ));
    }
    Ok(())
}

fn plugin_manifest(
    package: &PackageInventory,
    harness: &HarnessInventory,
    codex_interface: bool,
) -> Value {
    let mut value = json!({
        "name": package.name,
        "version": package.version,
        "description": package.description,
        "author": {"name": package.author},
        "homepage": package.homepage,
        "repository": package.repository,
        "license": package.license,
        "keywords": package.keywords,
        "skills": "./skills/",
        "mcpServers": format!("./{}", harness.mcp),
    });
    if codex_interface {
        value["interface"] = json!({
            "displayName": "Compass",
            "shortDescription": "Local-first structural code intelligence",
            "longDescription": package.description,
            "developerName": package.author,
            "category": "Developer Tools",
            "capabilities": ["Read"],
            "websiteURL": package.homepage,
            "defaultPrompt": [
                "Map this repository before making a cross-cutting change.",
                "Trace the callers and impact of this symbol."
            ]
        });
    }
    value
}

fn marketplace_manifest(package: &PackageInventory) -> Value {
    json!({
        "$schema": "https://json.schemastore.org/claude-code-marketplace.json",
        "name": "compass-plugins",
        "version": package.version,
        "description": package.description,
        "owner": {"name": package.author},
        "plugins": [{
            "name": package.name,
            "description": package.description,
            "version": package.version,
            "source": "./",
            "category": "development"
        }]
    })
}

fn codex_marketplace_manifest(package: &PackageInventory) -> Value {
    json!({
        "name": "compass-plugins",
        "interface": {"displayName": "Compass Plugins"},
        "plugins": [{
            "name": package.name,
            "source": {"source": "local", "path": "./"},
            "policy": {
                "installation": "AVAILABLE",
                "authentication": "ON_USE"
            },
            "category": "Developer Tools"
        }]
    })
}

fn opencode_manifest(
    package: &PackageInventory,
    harness: &HarnessInventory,
) -> Result<Value, String> {
    let name = harness
        .npm_name
        .as_deref()
        .ok_or_else(|| "error: OpenCode distribution is missing npm_name".to_owned())?;
    let api_version = harness
        .plugin_api_version
        .as_deref()
        .ok_or_else(|| "error: OpenCode distribution is missing plugin_api_version".to_owned())?;
    Ok(json!({
        "name": name,
        "version": package.version,
        "description": package.description,
        "license": package.license,
        "type": "module",
        "exports": "./src/index.js",
        "files": ["src/index.js", "skills", "opencode.json"],
        "dependencies": {"@opencode-ai/plugin": api_version}
    }))
}

fn mcp_config(
    platform: &str,
    transport: &str,
    opencode_plugin_name: Option<&str>,
) -> Result<Value, String> {
    let server = match transport {
        "stdio" => json!({"command": "compass", "args": ["serve", "--transport", "stdio"]}),
        "http" => json!({"type": "http", "url": "http://127.0.0.1:8080/mcp"}),
        _ => {
            return Err(format!(
                "error: unsupported distribution transport '{transport}'"
            ));
        }
    };
    Ok(if platform == "opencode" {
        let plugin_name = opencode_plugin_name
            .ok_or_else(|| "error: OpenCode distribution is missing npm_name".to_owned())?;
        let server = if transport == "stdio" {
            json!({"type": "local", "command": ["compass", "serve", "--transport", "stdio"], "enabled": true})
        } else {
            json!({"type": "remote", "url": "http://127.0.0.1:8080/mcp", "enabled": true})
        };
        json!({
            "$schema": "https://opencode.ai/config.json",
            "plugin": [plugin_name],
            "skills": {"paths": [format!("./node_modules/{plugin_name}/skills")]},
            "mcp": {"compass": server}
        })
    } else {
        json!({"mcpServers": {"compass": server}})
    })
}

fn insert_json(
    files: &mut BTreeMap<String, Vec<u8>>,
    path: &str,
    value: &Value,
) -> Result<(), String> {
    let mut encoded = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("error: could not encode {path}: {error}"))?;
    encoded.push(b'\n');
    files.insert(path.to_owned(), encoded);
    Ok(())
}

fn opencode_readme(package: &PackageInventory, harness: &HarnessInventory) -> String {
    format!(
        "# Compass for OpenCode\n\n{}\n\nVerified with OpenCode {}. The plugin registers a thin MCP configuration tool; graph logic remains in the `compass` binary.\n",
        package.description, harness.verified_version
    )
}
