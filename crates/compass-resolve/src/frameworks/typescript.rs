use ahash::AHashMap as HashMap;
use std::path::Path;

use compass_languages::{Extraction, FrameworkLimits};
use serde_json::Value;

use super::{FrameworkResolutionError, target_index::source_key};

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
    root: Option<&Path>,
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
        let source_file = source_key(source_file, root);
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
        let language = edge.attributes.get("language").and_then(Value::as_str);
        if !matches!(language, Some("python" | "javascript" | "typescript")) {
            continue;
        }
        let relation = edge.attributes.get("relation").and_then(Value::as_str);
        // Universal project resolution keeps module imports as one bounded
        // `imports_from` edge. The exact local binding is carried by the
        // source-backed reference/call edge (for example
        // `AccountAlias -> ./AccountPage::AccountPage`), so retain those
        // explicit binding facts for framework route targets too.
        let accepted_relation = match language {
            Some("python") => matches!(relation, Some("imports" | "imports_from" | "re_exports")),
            Some("javascript" | "typescript") => matches!(
                relation,
                Some(
                    "imports"
                        | "imports_from"
                        | "re_exports"
                        | "references"
                        | "calls"
                        | "constructs"
                )
            ),
            _ => false,
        };
        if !accepted_relation {
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
            .or_else(|| {
                matches!(language, Some("javascript" | "typescript"))
                    .then(|| edge.attributes.get("local_name"))
                    .flatten()
            })
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
        let (module, imported) = if matches!(language, Some("javascript" | "typescript")) {
            let module = edge
                .attributes
                .get("module")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .unwrap_or(qualified_target);
            let imported = edge
                .attributes
                .get("imported_name")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .or_else(|| {
                    edge.attributes
                        .get("binding_qualified_target")
                        .and_then(Value::as_str)
                        .and_then(|value| value.rsplit_once("::").map(|(_, name)| name))
                })
                .unwrap_or("*");
            (module.to_owned(), imported.to_owned())
        } else {
            qualified_target.rsplit_once('.').map_or_else(
                || (qualified_target.replace('.', "/"), "*".to_owned()),
                |(module, imported)| (module.replace('.', "/"), imported.to_owned()),
            )
        };
        let symbol_namespace = target_id.as_ref().and_then(|_| {
            module
                .rsplit('/')
                .next()
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        });
        let source_file = source_key(source_file, root);
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
