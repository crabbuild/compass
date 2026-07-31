use std::collections::HashMap;

use compass_languages::{Extraction, FrameworkLimits};
use serde_json::Value;

use super::FrameworkResolutionError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ImportAlias {
    pub module: String,
    pub imported: String,
    pub target_id: Option<String>,
    pub target_source: Option<String>,
}

pub(super) type ImportAliases = HashMap<(String, String), ImportAlias>;

pub(super) fn import_alias_map(
    extraction: &Extraction,
    limits: FrameworkLimits,
) -> Result<ImportAliases, FrameworkResolutionError> {
    let mut aliases = HashMap::new();
    let mut aliases_per_file = HashMap::new();
    for node in &extraction.nodes {
        let Some(source_file) = node
            .attributes
            .get("source_file")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let Some(local) = node
            .attributes
            .get("local_name")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let Some(module) = node
            .attributes
            .get("module")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let Some(imported) = node
            .attributes
            .get("imported_name")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let source_file = source_file.replace('\\', "/");
        insert_alias(
            &mut aliases,
            &mut aliases_per_file,
            limits,
            source_file,
            local,
            ImportAlias {
                module: module.to_owned(),
                imported: imported.to_owned(),
                target_id: None,
                target_source: None,
            },
        )?;
    }
    let nodes_by_id = extraction
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<HashMap<_, _>>();
    for edge in &extraction.edges {
        if edge.attributes.get("language").and_then(Value::as_str) != Some("python") {
            continue;
        }
        if !matches!(
            edge.attributes.get("relation").and_then(Value::as_str),
            Some("imports" | "imports_from" | "re_exports")
        ) {
            continue;
        }
        let Some(source_file) = edge
            .attributes
            .get("source_file")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let Some(local) = edge
            .attributes
            .get("binding_name")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let Some(qualified_target) = edge
            .attributes
            .get("binding_qualified_target")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let target = nodes_by_id.get(edge.target.as_str()).copied();
        let target_source = target
            .and_then(|node| node.attributes.get("source_file"))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(|value| value.replace('\\', "/"));
        let target_id = target
            .filter(|node| node.string("symbol_kind") != "file")
            .map(|node| node.id.clone());
        let (module, imported) = qualified_target.rsplit_once('.').map_or_else(
            || (qualified_target.replace('.', "/"), "*".to_owned()),
            |(module, imported)| (module.replace('.', "/"), imported.to_owned()),
        );
        let symbol_namespace = target_id.as_ref().and_then(|_| {
            module
                .rsplit('/')
                .next()
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        });
        let source_file = source_file.replace('\\', "/");
        insert_alias(
            &mut aliases,
            &mut aliases_per_file,
            limits,
            source_file.clone(),
            local,
            ImportAlias {
                module: module.clone(),
                imported: imported.clone(),
                target_id,
                target_source: target_source.clone(),
            },
        )?;
        if let Some(namespace) = symbol_namespace {
            insert_alias(
                &mut aliases,
                &mut aliases_per_file,
                limits,
                source_file,
                &namespace,
                ImportAlias {
                    module,
                    imported: "*".to_owned(),
                    target_id: None,
                    target_source,
                },
            )?;
        }
    }
    Ok(aliases)
}

fn insert_alias(
    aliases: &mut ImportAliases,
    aliases_per_file: &mut HashMap<String, usize>,
    limits: FrameworkLimits,
    source_file: String,
    local: &str,
    alias: ImportAlias,
) -> Result<(), FrameworkResolutionError> {
    let key = (source_file.clone(), local.to_owned());
    let is_new = !aliases.contains_key(&key);
    aliases.insert(key, alias);
    let aliases_in_file = aliases_per_file.entry(source_file).or_insert(0_usize);
    if is_new {
        *aliases_in_file += 1;
    }
    if *aliases_in_file > limits.max_alias_expansions {
        return Err(FrameworkResolutionError::AliasLimit {
            observed: *aliases_in_file,
            maximum: limits.max_alias_expansions,
        });
    }
    Ok(())
}
