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
        aliases.insert(
            (source_file.replace('\\', "/"), local.to_owned()),
            ImportAlias {
                module: module.to_owned(),
                imported: imported.to_owned(),
            },
        );
        if aliases.len() > limits.max_alias_expansions {
            return Err(FrameworkResolutionError::AliasLimit {
                observed: aliases.len(),
                maximum: limits.max_alias_expansions,
            });
        }
    }
    Ok(aliases)
}
