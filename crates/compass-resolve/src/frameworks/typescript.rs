use std::collections::HashMap;

use compass_languages::{Extraction, FrameworkLimits};
use serde_json::Value;

use super::FrameworkResolutionError;

pub(super) fn import_alias_map(
    extraction: &Extraction,
    limits: FrameworkLimits,
) -> Result<HashMap<String, String>, FrameworkResolutionError> {
    let mut aliases = HashMap::new();
    for node in &extraction.nodes {
        let Some(local) = node
            .attributes
            .get("local_name")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let Some(imported) = node
            .attributes
            .get("imported_name")
            .or_else(|| node.attributes.get("qualified_name"))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        aliases.insert(local.to_owned(), imported.to_owned());
        if aliases.len() > limits.max_alias_expansions {
            return Err(FrameworkResolutionError::AliasLimit {
                observed: aliases.len(),
                maximum: limits.max_alias_expansions,
            });
        }
    }
    Ok(aliases)
}
