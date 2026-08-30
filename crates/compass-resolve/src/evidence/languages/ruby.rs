//! Ruby-specific identity helpers used by the shared evidence resolver.

/// Split a Ruby owner/method identity without mistaking the `::` namespace
/// separator for the method-space separator.  Ruby instance methods use
/// `Owner#method`; singleton methods use `Owner.method`.
pub(crate) fn split_method_space(qualified: &str) -> Option<(&str, &str)> {
    let namespace = qualified.rfind("::").unwrap_or(0);
    let hash = qualified.rfind('#');
    let dot = qualified.rfind('.');
    match (hash, dot) {
        (Some(hash), Some(dot)) if hash.max(dot) > namespace => {
            let split = hash.max(dot);
            Some((&qualified[..split], &qualified[split + 1..]))
        }
        (Some(hash), None) if hash > namespace => {
            Some((&qualified[..hash], &qualified[hash + 1..]))
        }
        (None, Some(dot)) if dot > namespace => Some((&qualified[..dot], &qualified[dot + 1..])),
        _ => None,
    }
}

/// Return the receiver type portion of an instance or singleton Ruby
/// declaration.  The `::` namespace separator must be considered before the
/// method-space separators so `Billing::Invoice#save` remains one owner.
pub(crate) fn owner_type(qualified: &str) -> Option<&str> {
    let namespace = qualified.rfind("::").unwrap_or(0);
    let separator = qualified.rfind('#').max(qualified.rfind('.'))?;
    (separator > namespace).then_some(&qualified[..separator])
}

/// Enumerate the source-visible lexical constant candidates for a Ruby
/// receiver.  Ruby resolves a relative constant from the innermost lexical
/// owner outwards; the resolver later admits a result only when one exact
/// project declaration remains.  This keeps cross-file lookup precise without
/// falling back to terminal-name matching.
pub(crate) fn lexical_names(owner: &str, raw: &str) -> Vec<String> {
    let normalized = raw.trim().trim_start_matches("::");
    if normalized.is_empty() {
        return Vec::new();
    }
    if raw.trim().starts_with("::") {
        return vec![normalized.to_owned()];
    }
    let owner_type = owner_type(owner).unwrap_or_default();
    let parts = owner_type.split("::").collect::<Vec<_>>();
    let mut names = Vec::with_capacity(parts.len().saturating_add(1));
    for index in (0..=parts.len()).rev() {
        let prefix = parts[..index].join("::");
        let candidate = if prefix.is_empty() {
            normalized.to_owned()
        } else {
            format!("{prefix}::{normalized}")
        };
        if !names.iter().any(|existing| existing == &candidate) {
            names.push(candidate);
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::{lexical_names, split_method_space};

    #[test]
    fn method_space_split_preserves_ruby_namespaces() {
        assert_eq!(
            split_method_space("Billing::Invoice#save"),
            Some(("Billing::Invoice", "save"))
        );
        assert_eq!(
            split_method_space("Billing::Invoice.build"),
            Some(("Billing::Invoice", "build"))
        );
        assert_eq!(split_method_space("Billing::Invoice"), None);
    }

    #[test]
    fn lexical_names_preserve_nested_and_absolute_constant_lookup() {
        assert_eq!(
            lexical_names("Billing::CLI#run", "Environment"),
            vec![
                "Billing::CLI::Environment",
                "Billing::Environment",
                "Environment",
            ]
        );
        assert_eq!(
            lexical_names("Billing::CLI#run", "::Environment"),
            ["Environment"]
        );
    }
}
