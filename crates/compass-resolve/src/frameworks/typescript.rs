use std::collections::HashMap;

use compass_languages::{Extraction, FrameworkLimits};
use serde_json::Value;

use super::FrameworkResolutionError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ImportAlias {
    pub module: String,
    pub imported: String,
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
        let key = (source_file.clone(), local.to_owned());
        let is_new = !aliases.contains_key(&key);
        aliases.insert(
            key,
            ImportAlias {
                module: module.to_owned(),
                imported: imported.to_owned(),
            },
        );
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
    }
    Ok(aliases)
}
